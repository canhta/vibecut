//! Admits bounded control requests.
use super::{Caller, decode::decode_command, receipts::control_fingerprint};
use crate::Failure;
use crate::jobs::JobRunner;
use crate::session::wire::{
    API_VERSION, ControlResult, MAX_FRAME_BYTES, MAX_KEY_BYTES, MAX_PATH_BYTES, Payload, Request,
    Response, ResponseValue, bounded_json, parse_frame, registry_hash,
};
use crate::{
    Command, Effect, Lifecycle, Outcome, Session, control::Error as EngineError, control::Result,
};

impl Session {
    /// Parse a bounded UTF-8 frame, rejecting duplicate keys before dispatch.
    pub fn control_json(
        &mut self,
        bytes: &[u8],
        caller: Caller<'_>,
        runner: &mut JobRunner,
    ) -> Result<Response> {
        let value = parse_frame(bytes)?;
        let request = serde_json::from_value(value)
            .map_err(|_| EngineError::Control(crate::session::wire::ControlError::InvalidParams))?;
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
        let (response, _) = self.control_admitted(request, caller, runner)?;
        Ok(response)
    }

    /// Trusted in-process adapter entry: the same bounds, admission, receipts and
    /// events as [`Session::control_request`], but an execution refusal keeps its
    /// typed cause so the adapter can map the exit codes its own transport documents.
    /// Wire clients see only the closed [`ErrorCode`] on the response.
    pub fn control_typed(
        &mut self,
        request: Request,
        caller: Caller<'_>,
        runner: &mut JobRunner,
    ) -> Result<ControlResult> {
        match self.control_admitted(request, caller, runner)? {
            (_, Some(error)) => Err(error),
            (response, None) => match response.value {
                ResponseValue::Result(value) => Ok(value),
                // A replayed refusal carries only its wire code; the adapter maps it
                // as the generic refusal it is.
                ResponseValue::Error(_) => Err(EngineError::Edit(crate::EditError::Unsupported(
                    "replayed control refusal",
                ))),
            },
        }
    }

    /// Trusted in-process entry for a typed command: the process-lent access objects
    /// and the [`Policy`] authorization of the full command tree apply, but the
    /// transport envelope (request ids, registry hash, base revision, idempotency
    /// receipts, response events) does not exist for a caller that holds a [`Command`].
    pub fn control_command(
        &mut self,
        command: Command,
        caller: Caller<'_>,
        runner: &mut JobRunner,
    ) -> Result<ControlResult> {
        let mut pending = vec![&command];
        while let Some(child) = pending.pop() {
            caller.policy.authorize(child, self.project())?;
            if let Command::Batch { commands } | Command::TimelineEdit { commands, .. } = child {
                pending.extend(commands);
            }
        }
        let fingerprint = blake3::hash(
            &serde_json::to_vec(&serde_json::json!({
                "command": &command,
                "base_rev": self.revision(),
                "session_id": self.session_id(),
            }))
            .map_err(|_| EngineError::Control(crate::session::wire::ControlError::InvalidParams))?,
        );
        self.run_control(command, runner, fingerprint, &caller)
    }

    fn control_admitted(
        &mut self,
        request: Request,
        caller: Caller<'_>,
        runner: &mut JobRunner,
    ) -> Result<(Response, Option<EngineError>)> {
        crate::session::budget(&request.params)?;
        let frame = bounded_json(&request, MAX_FRAME_BYTES)?;
        parse_frame(&frame)?;
        if request.request_id.is_empty()
            || request.request_id.len() > MAX_KEY_BYTES
            || caller.principal.is_empty()
            || caller.principal.len() > MAX_KEY_BYTES
        {
            return Err(EngineError::Control(
                crate::session::wire::ControlError::Limit,
            ));
        }
        let before = self.revision();
        match self.admit_and_run(&request, &caller, runner) {
            Ok(admitted) => Ok(admitted),
            Err(error) => {
                let code = error.code();
                self.ledger.push(
                    &self.identity.0,
                    self.revision(),
                    Payload::CommandError {
                        request_id: request.request_id.clone(),
                        code,
                    },
                );
                Ok((
                    Response {
                        api_version: API_VERSION,
                        registry_hash: registry_hash().into(),
                        session_id: self.session_id().into(),
                        request_id: request.request_id,
                        rev_before: before,
                        rev_after: self.revision(),
                        value: ResponseValue::Error(code),
                    },
                    Some(error),
                ))
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
    ) -> Result<(Response, Option<EngineError>)> {
        if request.api_version != API_VERSION {
            return Err(EngineError::Control(
                crate::session::wire::ControlError::SchemaMismatch,
            ));
        }
        if request.session_id != self.session_id() {
            return Err(EngineError::Control(
                crate::session::wire::ControlError::SessionGone,
            ));
        }
        let command = decode_command(&request.command, request.params.clone())?;
        let effectful = command.effect() != Effect::Read;
        if (effectful || request.registry_hash.is_some())
            && request.registry_hash.as_deref() != Some(registry_hash())
        {
            return Err(EngineError::Control(
                crate::session::wire::ControlError::SchemaMismatch,
            ));
        }
        if caller.external
            && matches!(
                command,
                Command::BeginGesture {}
                    | Command::CommitGesture {}
                    | Command::CancelGesture {}
                    | Command::TimelinePreview { .. }
                    | Command::TimelineCommitPreview {}
                    | Command::TimelineDiscardPreview {}
            )
        {
            return Err(EngineError::Control(
                crate::session::wire::ControlError::Permission,
            ));
        }
        // A generic run and every nested child get the same trusted policy.
        let mut pending = vec![&command];
        while let Some(child) = pending.pop() {
            caller.policy.authorize(child, self.project())?;
            if let Command::Batch { commands } | Command::TimelineEdit { commands, .. } = child {
                pending.extend(commands);
            }
        }
        let fingerprint = control_fingerprint(request)?;
        if let Command::Export { path, .. }
        | Command::ExportSubtitles { path, .. }
        | Command::BurnSubtitles { path, .. }
        | Command::Snapshot { path, .. }
        | Command::ExportProjectBundle { path }
        | Command::SaveProjectAs { path }
        | Command::ClipsExport {
            directory: path, ..
        }
        | Command::ExportClips { dir: path, .. } = &command
        {
            bounded_json(path, MAX_PATH_BYTES)?;
        }
        if let Command::ImportProjectBundle {
            bundle,
            destination,
        } = &command
        {
            bounded_json(bundle, MAX_PATH_BYTES)?;
            bounded_json(destination, MAX_PATH_BYTES)?;
        }
        let key = if effectful {
            let key = request
                .idempotency_key
                .as_ref()
                .ok_or(EngineError::Control(
                    crate::session::wire::ControlError::InvalidParams,
                ))?;
            if key.is_empty() || key.len() > MAX_KEY_BYTES {
                return Err(EngineError::Control(
                    crate::session::wire::ControlError::Limit,
                ));
            }
            let key = (caller.principal.to_owned(), key.clone());
            if let Some(response) = self.replay_receipt(&key, fingerprint, &request.request_id)? {
                return Ok((response, None));
            }
            self.check_receipt_capacity()?;
            if request.base_rev.as_ref() != Some(&self.revision()) {
                return Err(EngineError::Control(
                    crate::session::wire::ControlError::StaleRevision,
                ));
            }
            if self.transaction_active() && caller.external {
                return Err(EngineError::Session(
                    crate::session::SessionError::TransactionActive,
                ));
            }
            Some(key)
        } else {
            None
        };
        let before = self.revision();
        // All mutation response shapes have a fixed small bound (at most 1024
        // created IDs). Reads/images never occupy the no-eviction receipt ledger.
        let (value, failure) = match self.run_control(command, runner, fingerprint, caller) {
            Ok(value) => (ResponseValue::Result(value), None),
            Err(error) => (ResponseValue::Error(error.code()), Some(error)),
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
        Ok((response, failure))
    }
}

impl Session {
    /// The one transport entry: a lifecycle save is admitted by
    /// `control_lifecycle_save` and publishes through `publish`; every other request
    /// goes through `control_request`. `publish` is unused for non-save requests.
    pub fn control_routed(
        &mut self,
        request: Request,
        caller: Caller<'_>,
        runner: &mut JobRunner,
        publish: impl FnOnce(&project::Project, &project::Metadata) -> Result<()>,
    ) -> Result<Response> {
        match crate::lifecycle(&request.command) {
            Some(Lifecycle::Save) => self.control_lifecycle_save(request, caller, publish),
            Some(
                Lifecycle::Create
                | Lifecycle::Duplicate
                | Lifecycle::Delete
                | Lifecycle::Restore
                | Lifecycle::Purge
                | Lifecycle::Thumbnail
                | Lifecycle::SaveAs
                | Lifecycle::ExportBundle
                | Lifecycle::ImportBundle,
            )
            | None => self.control_request(request, caller, runner),
        }
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
            return Err(EngineError::Control(
                crate::session::wire::ControlError::Limit,
            ));
        }
        let before = self.revision();
        match self.admit_lifecycle_save(&request, &caller, publish) {
            Ok(response) => Ok(response),
            Err(error) => {
                let code = error.code();
                self.ledger.push(
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
            return Err(EngineError::Control(
                crate::session::wire::ControlError::SchemaMismatch,
            ));
        }
        if request.session_id != self.session_id() {
            return Err(EngineError::Control(
                crate::session::wire::ControlError::SessionGone,
            ));
        }
        let command = decode_command(&request.command, request.params.clone())?;
        if !matches!(command, Command::SaveProject {}) {
            return Err(EngineError::Control(
                crate::session::wire::ControlError::InvalidParams,
            ));
        }
        if request.registry_hash.as_deref() != Some(registry_hash()) {
            return Err(EngineError::Control(
                crate::session::wire::ControlError::SchemaMismatch,
            ));
        }
        caller.policy.authorize(&command, self.project())?;
        let fingerprint = control_fingerprint(request)?;
        let key = request
            .idempotency_key
            .as_ref()
            .ok_or(EngineError::Control(
                crate::session::wire::ControlError::InvalidParams,
            ))?;
        if key.is_empty() || key.len() > MAX_KEY_BYTES {
            return Err(EngineError::Control(
                crate::session::wire::ControlError::Limit,
            ));
        }
        let key = (caller.principal.to_owned(), key.clone());
        if let Some(response) = self.replay_receipt(&key, fingerprint, &request.request_id)? {
            return Ok(response);
        }
        self.check_receipt_capacity()?;
        if request.base_rev.as_ref() != Some(&self.revision()) {
            return Err(EngineError::Control(
                crate::session::wire::ControlError::StaleRevision,
            ));
        }
        if self.transaction_active() {
            return Err(EngineError::Session(
                crate::session::SessionError::TransactionActive,
            ));
        }
        let before = self.revision();
        let mut metadata = self.metadata().clone();
        metadata.touch();
        let outcome = match publish(self.project(), &metadata) {
            Ok(()) => match self.restore_metadata(metadata) {
                Ok(()) => self.saved().map_err(EngineError::from),
                Err(error) => Err(error.into()),
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
            Err(error) => ResponseValue::Error(error.code()),
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
