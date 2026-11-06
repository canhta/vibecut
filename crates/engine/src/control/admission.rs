//! Admits bounded control requests.
use super::{
    API_VERSION, BTreeMap, Caller, Capped, Command, ControlResult, EditError, Effect, ErrorCode,
    JobRunner, JsonSeed, MAX_COLLECTION, MAX_DEPTH, MAX_FRAME_BYTES, MAX_KEY_BYTES, MAX_NODES,
    MAX_PATH_BYTES, MAX_RESPONSE_BYTES, MAX_SAFE_INTEGER, MAX_TEXT_BYTES, OnceLock, Outcome,
    Payload, Request, Response, ResponseValue, Result, Serialize, Session, Value, Write,
    control_fingerprint, json, registry, registry_hash,
};
/// Validate the actual wire representation of a candidate, including generated
/// IDs and arithmetic-derived ticks, while the edit is still reversible.
pub(crate) fn validate_wire<T: Serialize + ?Sized>(value: &T) -> Result<()> {
    let bytes = bounded_json(value, MAX_RESPONSE_BYTES - 4096)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| EditError::InvalidParams)?;
    check_integers(&value)
}

fn check_integers(value: &Value) -> Result<()> {
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        match value {
            Value::Number(n)
                if n.as_i64()
                    .is_some_and(|v| !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&v))
                    || n.as_u64().is_some_and(|v| v > MAX_SAFE_INTEGER as u64) =>
            {
                return Err(EditError::InvalidParams);
            }
            Value::Array(values) => stack.extend(values),
            Value::Object(values) => stack.extend(values.values()),
            _ => {}
        }
    }
    Ok(())
}

pub(crate) fn decode_command(id: &str, params: Value) -> Result<Command> {
    crate::session::budget(&params)?;
    type Validators = BTreeMap<&'static str, jsonschema::Validator>;
    static VALIDATORS: OnceLock<std::result::Result<Validators, ()>> = OnceLock::new();
    let validators = VALIDATORS
        .get_or_init(|| {
            registry()
                .iter()
                .map(|s| {
                    jsonschema::options()
                        .offline()
                        .build(&s.params_schema)
                        .map(|v| (s.id, v))
                        .map_err(|_| ())
                })
                .collect()
        })
        .as_ref()
        .map_err(|_| EditError::SchemaMismatch)?;
    let validator = validators.get(id).ok_or(EditError::UnknownCommand)?;
    if !validator.is_valid(&params) {
        return Err(EditError::InvalidParams);
    }
    let command: Command = serde_json::from_value(json!({"command":id,"params":params}))
        .map_err(|_| EditError::InvalidParams)?;
    // Includes nested publications/session controls, before any first child effect.
    if matches!(command, Command::Batch { .. }) {
        crate::session::check_budget(&command)?;
    }
    Ok(command)
}

impl Session {
    /// Parse a bounded UTF-8 frame, rejecting duplicate keys before dispatch.
    pub fn control_json(
        &mut self,
        bytes: &[u8],
        caller: Caller<'_>,
        runner: &mut JobRunner,
    ) -> Result<Response> {
        let value = parse_frame(bytes)?;
        let request = serde_json::from_value(value).map_err(|_| EditError::InvalidParams)?;
        self.control_request(request, caller, runner)
    }
}

impl Session {
    /// Single mutable Session ownership serializes admission, key reservation,
    /// revision comparison and effects. Callers must not unlock between them.
    pub fn control_request(
        &mut self,
        request: Request,
        caller: Caller<'_>,
        runner: &mut JobRunner,
    ) -> Result<Response> {
        crate::session::budget(&request.params)?;
        let frame = bounded_json(&request, MAX_FRAME_BYTES)?;
        parse_frame(&frame)?;
        if request.request_id.is_empty()
            || request.request_id.len() > MAX_KEY_BYTES
            || caller.principal.is_empty()
            || caller.principal.len() > MAX_KEY_BYTES
        {
            return Err(EditError::Limit);
        }
        let before = self.revision();
        let result = self.admit_and_run(&request, &caller, runner);
        match result {
            Ok(response) => Ok(response),
            Err(error) => {
                let code = ErrorCode::from_error(&error);
                self.control.push(
                    &self.identity.0,
                    self.revision(),
                    Payload::CommandError {
                        request_id: request.request_id.clone(),
                        code,
                    },
                );
                Ok(Response {
                    api_version: API_VERSION,
                    registry_hash: registry_hash().into(),
                    session_id: self.session_id().into(),
                    request_id: request.request_id,
                    rev_before: before,
                    rev_after: self.revision(),
                    value: ResponseValue::Error(code),
                })
            }
        }
    }
}

impl Session {
    pub(super) fn admit_and_run(
        &mut self,
        request: &Request,
        caller: &Caller<'_>,
        runner: &mut JobRunner,
    ) -> Result<Response> {
        if request.api_version != API_VERSION {
            return Err(EditError::SchemaMismatch);
        }
        if request.session_id != self.session_id() {
            return Err(EditError::SessionGone);
        }
        let command = decode_command(&request.command, request.params.clone())?;
        let effectful = command.effect() != Effect::Read;
        if (effectful || request.registry_hash.is_some())
            && request.registry_hash.as_deref() != Some(registry_hash())
        {
            return Err(EditError::SchemaMismatch);
        }
        if caller.external
            && matches!(
                command,
                Command::BeginGesture {} | Command::CommitGesture {} | Command::CancelGesture {}
            )
        {
            return Err(EditError::Permission);
        }
        // A generic run and every nested child get the same trusted policy.
        let mut pending = vec![&command];
        while let Some(child) = pending.pop() {
            caller.policy.authorize(child, self.project())?;
            if let Command::Batch { commands } = child {
                pending.extend(commands);
            }
        }
        let fingerprint = control_fingerprint(request)?;
        if let Command::Export { path, .. }
        | Command::ExportSubtitles { path, .. }
        | Command::BurnSubtitles { path, .. }
        | Command::Snapshot { path, .. } = &command
        {
            bounded_json(path, MAX_PATH_BYTES)?;
        }
        let key = if effectful {
            let key = request
                .idempotency_key
                .as_ref()
                .ok_or(EditError::InvalidParams)?;
            if key.is_empty() || key.len() > MAX_KEY_BYTES {
                return Err(EditError::Limit);
            }
            let key = (caller.principal.to_owned(), key.clone());
            if let Some(response) = self.replay_receipt(&key, fingerprint, &request.request_id)? {
                return Ok(response);
            }
            self.check_receipt_capacity()?;
            if request.base_rev.as_ref() != Some(&self.revision()) {
                return Err(EditError::StaleRevision);
            }
            if self.transaction_active() && caller.external {
                return Err(EditError::TransactionActive);
            }
            Some(key)
        } else {
            None
        };
        let before = self.revision();
        // All mutation response shapes have a fixed small bound (at most 1024
        // created IDs). Reads/images never occupy the no-eviction receipt ledger.
        let value = match self.run_control(command, runner, fingerprint, caller.policy) {
            Ok(value) => ResponseValue::Result(value),
            Err(error) => ResponseValue::Error(ErrorCode::from_error(&error)),
        };
        let response = Response {
            api_version: API_VERSION,
            registry_hash: registry_hash().into(),
            session_id: self.session_id().into(),
            request_id: request.request_id.clone(),
            rev_before: before,
            rev_after: self.revision(),
            value,
        };
        if let Some(key) = key {
            self.store_receipt(key, fingerprint, &response)?;
        }
        self.push_control_event(&request.request_id, &response);
        Ok(response)
    }
}

impl Session {
    /// Admit and acknowledge one composition-root lifecycle save through the same
    /// envelope, policy, revision and receipt rules as ordinary effects. The
    /// callback performs only project/metadata file publication; the engine owns
    /// admission and the receipt, so replay, conflicts, ID bounds and capacity do
    /// not vary by command. Once execution starts, success and typed execution
    /// errors are both receipted; the composition root keeps its dirty state on
    /// failure and a retry that must execute again uses a fresh key.
    pub fn control_lifecycle_save(
        &mut self,
        request: Request,
        caller: Caller<'_>,
        publish: impl FnOnce(&project::Project, &project::Metadata) -> Result<()>,
    ) -> Result<Response> {
        crate::session::budget(&request.params)?;
        let frame = bounded_json(&request, MAX_FRAME_BYTES)?;
        parse_frame(&frame)?;
        if request.request_id.is_empty()
            || request.request_id.len() > MAX_KEY_BYTES
            || caller.principal.is_empty()
            || caller.principal.len() > MAX_KEY_BYTES
        {
            return Err(EditError::Limit);
        }
        let before = self.revision();
        match self.admit_lifecycle_save(&request, &caller, publish) {
            Ok(response) => Ok(response),
            Err(error) => {
                let code = ErrorCode::from_error(&error);
                self.control.push(
                    &self.identity.0,
                    self.revision(),
                    Payload::CommandError {
                        request_id: request.request_id.clone(),
                        code,
                    },
                );
                Ok(Response {
                    api_version: API_VERSION,
                    registry_hash: registry_hash().into(),
                    session_id: self.session_id().into(),
                    request_id: request.request_id,
                    rev_before: before,
                    rev_after: self.revision(),
                    value: ResponseValue::Error(code),
                })
            }
        }
    }
}

impl Session {
    pub(super) fn admit_lifecycle_save(
        &mut self,
        request: &Request,
        caller: &Caller<'_>,
        publish: impl FnOnce(&project::Project, &project::Metadata) -> Result<()>,
    ) -> Result<Response> {
        if request.api_version != API_VERSION {
            return Err(EditError::SchemaMismatch);
        }
        if request.session_id != self.session_id() {
            return Err(EditError::SessionGone);
        }
        let command = decode_command(&request.command, request.params.clone())?;
        if !matches!(command, Command::SaveProject {}) {
            return Err(EditError::InvalidParams);
        }
        if request.registry_hash.as_deref() != Some(registry_hash()) {
            return Err(EditError::SchemaMismatch);
        }
        caller.policy.authorize(&command, self.project())?;
        let fingerprint = control_fingerprint(request)?;
        let key = request
            .idempotency_key
            .as_ref()
            .ok_or(EditError::InvalidParams)?;
        if key.is_empty() || key.len() > MAX_KEY_BYTES {
            return Err(EditError::Limit);
        }
        let key = (caller.principal.to_owned(), key.clone());
        if let Some(response) = self.replay_receipt(&key, fingerprint, &request.request_id)? {
            return Ok(response);
        }
        self.check_receipt_capacity()?;
        if request.base_rev.as_ref() != Some(&self.revision()) {
            return Err(EditError::StaleRevision);
        }
        if self.transaction_active() {
            return Err(EditError::TransactionActive);
        }
        let before = self.revision();
        let mut metadata = self.metadata().clone();
        metadata.touch();
        let outcome = match publish(self.project(), &metadata) {
            Ok(()) => match self.restore_metadata(metadata) {
                Ok(()) => self.saved(),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        let value = match outcome {
            Ok(()) => ResponseValue::Result(ControlResult::Edit {
                outcome: Outcome {
                    changed: false,
                    created: Vec::new(),
                    placements: Vec::new(),
                },
            }),
            Err(error) => ResponseValue::Error(ErrorCode::from_error(&error)),
        };
        let response = Response {
            api_version: API_VERSION,
            registry_hash: registry_hash().into(),
            session_id: self.session_id().into(),
            request_id: request.request_id.clone(),
            rev_before: before,
            rev_after: self.revision(),
            value,
        };
        self.store_receipt(key, fingerprint, &response)?;
        self.push_control_event(&request.request_id, &response);
        Ok(response)
    }
}
impl Write for Capped {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.bytes.len().saturating_add(bytes.len()) > self.max {
            return Err(std::io::Error::other("control byte limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub fn bounded_json<T: Serialize + ?Sized>(value: &T, max: usize) -> Result<Vec<u8>> {
    let mut writer = Capped {
        bytes: Vec::new(),
        max: max.min(MAX_RESPONSE_BYTES),
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| EditError::Limit)?;
    Ok(writer.bytes)
}

pub fn parse_frame(bytes: &[u8]) -> Result<Value> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(EditError::Limit);
    }
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = serde::de::DeserializeSeed::deserialize(
        JsonSeed {
            depth: 0,
            nodes: &mut 0,
            text: &mut 0,
        },
        &mut deserializer,
    )
    .map_err(|_| EditError::InvalidParams)?;
    deserializer.end().map_err(|_| EditError::InvalidParams)?;
    Ok(value)
}
impl<'de> serde::de::DeserializeSeed<'de> for JsonSeed<'_> {
    type Value = Value;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        d: D,
    ) -> std::result::Result<Value, D::Error> {
        if self.depth > MAX_DEPTH || *self.nodes >= MAX_NODES {
            return Err(serde::de::Error::custom("control limit"));
        }
        *self.nodes += 1;
        d.deserialize_any(self)
    }
}
impl<'de> serde::de::Visitor<'de> for JsonSeed<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("bounded JSON")
    }
    fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_bool<E: serde::de::Error>(self, v: bool) -> std::result::Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Value, E> {
        if !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&v) {
            return Err(E::custom("integer range"));
        }
        Ok(json!(v))
    }
    fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Value, E> {
        if v > MAX_SAFE_INTEGER as u64 {
            return Err(E::custom("integer range"));
        }
        Ok(json!(v))
    }
    fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Value, E> {
        if !v.is_finite() || v.abs() > MAX_SAFE_INTEGER as f64 {
            return Err(E::custom("number range"));
        }
        Ok(json!(v))
    }
    fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Value, E> {
        *self.text = self.text.saturating_add(v.len());
        if *self.text > MAX_TEXT_BYTES {
            return Err(E::custom("text limit"));
        }
        Ok(Value::String(v.into()))
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(
        self,
        mut a: A,
    ) -> std::result::Result<Value, A::Error> {
        let mut values = serde_json::Map::new();
        while let Some(key) = a.next_key::<String>()? {
            *self.text = self.text.saturating_add(key.len());
            if values.len() >= MAX_COLLECTION
                || *self.text > MAX_TEXT_BYTES
                || values.contains_key(&key)
            {
                return Err(serde::de::Error::custom("object limit or duplicate key"));
            }
            let value = a.next_value_seed(JsonSeed {
                depth: self.depth + 1,
                nodes: self.nodes,
                text: self.text,
            })?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(
        self,
        mut a: A,
    ) -> std::result::Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(v) = a.next_element_seed(JsonSeed {
            depth: self.depth + 1,
            nodes: self.nodes,
            text: self.text,
        })? {
            if values.len() >= MAX_COLLECTION {
                return Err(serde::de::Error::custom("array limit"));
            }
            values.push(v);
        }
        Ok(Value::Array(values))
    }
}
