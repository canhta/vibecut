//! Transport-neutral control admission, session receipts and snapshot resynchronization.
pub use crate::control_preview::PreviewResult;
use crate::jobs::{JobId, JobKind, JobRunner, JobSnapshot, JobStage, JobState};
use crate::{Command, EditError, Effect, Outcome, Result, Session, registry};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    io::Write,
    sync::OnceLock,
};

pub const API_VERSION: u32 = 1;
pub const MAX_FRAME_BYTES: usize = 8 << 20;
pub const MAX_RESPONSE_BYTES: usize = 8 << 20;
pub const MAX_RECEIPTS: usize = 4096;
pub const MAX_RECEIPT_BYTES: usize = 32 << 20;
pub const RECEIPT_RESERVATION: usize = 128 << 10;
pub const MAX_EVENTS: usize = 128;
pub const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

pub const MAX_DEPTH: usize = 32;
pub const MAX_NODES: usize = 200_000;
pub const MAX_COLLECTION: usize = 100_000;
pub const MAX_TEXT_BYTES: usize = 4 << 20;
pub const MAX_BATCH_COMMANDS: usize = 1024;
pub const MAX_BATCH_DEPTH: usize = 16;
pub const MAX_KEY_BYTES: usize = 128;
pub const MAX_PATH_BYTES: usize = 4096;
pub const MAX_PREVIEW_FRAMES: usize = 4;
pub const MAX_PREVIEW_EDGE: u32 = 640;
pub const PREVIEW_QUALITY: u8 = 80;
pub const MAX_PREVIEW_CLIPS: usize = 16;

/// Static admission budgets travel with the catalog and participate in its hash.
#[derive(Clone, Copy, Debug, Serialize, schemars::JsonSchema)]
pub struct Limits {
    pub frame_bytes: usize,
    pub response_bytes: usize,
    pub depth: usize,
    pub nodes: usize,
    pub collection: usize,
    pub text_bytes: usize,
    pub batch_commands: usize,
    pub batch_depth: usize,
    pub key_bytes: usize,
    pub path_bytes: usize,
    pub receipts: usize,
    pub receipt_bytes: usize,
    pub receipt_reservation: usize,
    pub events: usize,
    pub preview_frames: usize,
    pub preview_edge: u32,
    pub preview_quality: u8,
    pub preview_clips: usize,
    pub safe_integer: i64,
}
pub const LIMITS: Limits = Limits {
    frame_bytes: MAX_FRAME_BYTES,
    response_bytes: MAX_RESPONSE_BYTES,
    depth: MAX_DEPTH,
    nodes: MAX_NODES,
    collection: MAX_COLLECTION,
    text_bytes: MAX_TEXT_BYTES,
    batch_commands: MAX_BATCH_COMMANDS,
    batch_depth: MAX_BATCH_DEPTH,
    key_bytes: MAX_KEY_BYTES,
    path_bytes: MAX_PATH_BYTES,
    receipts: MAX_RECEIPTS,
    receipt_bytes: MAX_RECEIPT_BYTES,
    receipt_reservation: RECEIPT_RESERVATION,
    events: MAX_EVENTS,
    preview_frames: MAX_PREVIEW_FRAMES,
    preview_edge: MAX_PREVIEW_EDGE,
    preview_quality: PREVIEW_QUALITY,
    preview_clips: MAX_PREVIEW_CLIPS,
    safe_integer: MAX_SAFE_INTEGER,
};

/// Opaque epoch/generation; never restored by undo or exposed as an address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct Revision(pub(crate) String);

#[derive(Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub api_version: u32,
    pub registry_hash: Option<String>,
    pub session_id: String,
    pub request_id: String,
    pub idempotency_key: Option<String>,
    pub base_rev: Option<Revision>,
    pub command: String,
    pub params: Value,
}
/// Supplied by the trusted composition root, never deserialized from client JSON.
/// Implementations admit source reads, destination writes/overwrite and provider
/// consent through their owners. The engine calls this on every child before IO.
pub trait Policy {
    fn authorize(&self, command: &Command, project: &project::Project) -> Result<()>;
}
pub struct Caller<'a> {
    pub principal: &'a str,
    pub external: bool,
    pub policy: &'a dyn Policy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    UnknownCommand,
    InvalidParams,
    Limit,
    SchemaMismatch,
    SessionGone,
    StaleRevision,
    Permission,
    IdempotencyConflict,
    ReceiptCapacity,
    ResyncRequired,
    JobNotRetained,
    Cancelled,
    QueueFull,
    TransactionActive,
    Refused,
    Failed,
}
impl ErrorCode {
    pub fn from_error(error: &EditError) -> Self {
        match error {
            EditError::UnknownCommand => Self::UnknownCommand,
            EditError::InvalidParams => Self::InvalidParams,
            EditError::Limit | EditError::ArtifactTooLarge | EditError::BudgetFull => Self::Limit,
            EditError::SchemaMismatch => Self::SchemaMismatch,
            EditError::SessionGone => Self::SessionGone,
            EditError::StaleRevision | EditError::StaleResult => Self::StaleRevision,
            EditError::Permission | EditError::ConsentRequired => Self::Permission,
            EditError::IdempotencyConflict => Self::IdempotencyConflict,
            EditError::ReceiptCapacity => Self::ReceiptCapacity,
            EditError::ResyncRequired => Self::ResyncRequired,
            EditError::JobNotRetained => Self::JobNotRetained,
            EditError::Cancelled => Self::Cancelled,
            EditError::QueueFull => Self::QueueFull,
            EditError::TransactionActive | EditError::NoGesture => Self::TransactionActive,
            EditError::Panicked | EditError::WorkerPanicked | EditError::Io(_) => Self::Failed,
            _ => Self::Refused,
        }
    }
}
#[derive(Clone, Serialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlResult {
    Edit {
        outcome: Outcome,
    },
    Inspection {
        project: project::ProjectData,
    },
    Catalog {
        entries: Vec<crate::CommandSpec>,
    },
    Providers {
        entries: Vec<ProviderView>,
    },
    JobAccepted {
        job: JobId,
        revision: Revision,
        paths: Vec<std::path::PathBuf>,
    },
    Job {
        snapshot: JobView,
    },
    CancelRequested {
        job: JobId,
    },
}
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
pub struct ProviderView {
    pub operation: String,
    pub provider: String,
    pub models: Vec<String>,
    pub remote: bool,
    pub native_timing: bool,
    pub languages: Vec<String>,
    pub target_languages: Vec<String>,
}
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("api_version", &self.api_version)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for ControlResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self {
            Self::Edit { .. } => "edit",
            Self::Inspection { .. } => "inspection",
            Self::Catalog { .. } => "catalog",
            Self::Providers { .. } => "providers",
            Self::JobAccepted { .. } => "job_accepted",
            Self::Job { .. } => "job",
            Self::CancelRequested { .. } => "cancel_requested",
        };
        f.debug_struct("ControlResult")
            .field("kind", &kind)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for PreviewFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewFrame")
            .field("frame", &self.frame)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}
impl From<crate::subtitles::ProviderOption> for ProviderView {
    fn from(p: crate::subtitles::ProviderOption) -> Self {
        Self {
            operation: match p.operation {
                crate::subtitles::CatalogOperation::Transcribe => "transcribe",
                crate::subtitles::CatalogOperation::Translate => "translate",
            }
            .into(),
            provider: p.id.into(),
            models: p.models,
            remote: !p.local,
            native_timing: p.native_timing,
            languages: p.languages,
            target_languages: p.target_languages,
        }
    }
}
#[derive(Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PreviewFrame {
    pub frame: i64,
    pub output_tick: time::Tick,
    pub clips: Vec<project::ItemId>,
    pub width: u32,
    pub height: u32,
    /// JPEG bytes, encoded at quality 80; transports choose their binary encoding.
    pub jpeg: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
pub struct JobView {
    pub id: JobId,
    pub kind: JobKind,
    pub stage: JobStage,
    pub progress: f32,
    pub state: JobState,
    pub attempts: u8,
    pub error: Option<ErrorCode>,
    pub result: Option<PreviewResult>,
}
impl From<JobSnapshot> for JobView {
    fn from(s: JobSnapshot) -> Self {
        Self {
            id: s.id,
            kind: s.kind,
            stage: s.stage,
            progress: s.progress,
            state: s.state,
            attempts: s.attempts,
            error: s.error.as_deref().map(ErrorCode::from_error),
            result: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
pub enum ResponseValue {
    Result(ControlResult),
    Error(ErrorCode),
}
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
pub struct Response {
    pub api_version: u32,
    pub registry_hash: String,
    pub session_id: String,
    pub request_id: String,
    pub rev_before: Revision,
    pub rev_after: Revision,
    pub value: ResponseValue,
}
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Payload {
    CommandResult {
        request_id: String,
        result: ControlResult,
    },
    CommandError {
        request_id: String,
        code: ErrorCode,
    },
    ProjectChanged {
        full_snapshot_required: bool,
    },
    RevisionChanged,
    JobChanged {
        snapshot: JobView,
    },
}
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
pub struct Event {
    pub api_version: u32,
    pub registry_hash: String,
    pub session_id: String,
    pub seq: String,
    pub revision: Revision,
    pub payload: Payload,
}
/// A cursor holds no queue or project. The bounded engine window is shared by all
/// listeners; gaps are explicit refusals, never a promise of durable replay.
#[derive(Clone, Debug)]
pub struct Cursor {
    session: String,
    seq: u64,
}
pub struct Snapshot {
    pub project: std::sync::Arc<project::Project>,
    pub revision: Revision,
    pub jobs: Vec<JobView>,
    pub cursor: Cursor,
}
struct Receipt {
    fingerprint: blake3::Hash,
    response: Response,
}
#[derive(Default)]
pub(crate) struct State {
    receipts: BTreeMap<(String, String), Receipt>,
    receipt_bytes: usize,
    events: VecDeque<std::sync::Arc<Event>>,
    sequence: u64,
    jobs: BTreeMap<JobId, JobView>,
}
impl State {
    pub(crate) fn push(&mut self, session: &str, revision: Revision, payload: Payload) {
        // A saturated stream can never manufacture another sequence: all cursors
        // require resync. This is unreachable in a practical Session lifetime.
        let Some(next) = self.sequence.checked_add(1) else {
            self.events.clear();
            return;
        };
        self.sequence = next;
        self.events.push_back(std::sync::Arc::new(Event {
            api_version: API_VERSION,
            registry_hash: registry_hash().into(),
            session_id: session.into(),
            seq: next.to_string(),
            revision,
            payload,
        }));
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
    }
}

/// Deterministic build-time reference from the executable registry.
pub fn reference_markdown() -> Result<String> {
    let mut out = format!(
        "# Engine commands\n\nGenerated by `cargo run -p engine --example control_catalog`; do not edit.\n\nAPI version: {API_VERSION}. Registry digest: `{}`.\n\n",
        registry_hash()
    );
    for spec in registry() {
        out.push_str(&format!(
            "## `{}` — {}\n\nEffect: `{:?}`. Fixture: `{:?}`.\n\n```json\n{}\n```\n\n",
            spec.id,
            spec.label,
            spec.effect,
            spec.example.fixture,
            serde_json::to_string_pretty(&spec.example.params)
                .map_err(|_| EditError::InvalidParams)?
        ));
    }
    let _ = out.pop();
    Ok(out)
}

pub fn contract_schema() -> Value {
    let serialize = || {
        schemars::generate::SchemaSettings::draft2020_12()
            .for_serialize()
            .into_generator()
    };
    let mut request = schemars::schema_for!(Request).to_value();
    close_schema(&mut request);
    let mut response = serialize().into_root_schema_for::<Response>().to_value();
    close_schema(&mut response);
    let mut event = serialize().into_root_schema_for::<Event>().to_value();
    close_schema(&mut event);
    json!({"request":request,"response":response,"event":event})
}

pub fn registry_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"vibecut-control-v1\0");
        // The generated registry contains no dynamic availability/state/timestamps.
        if let Ok(bytes) =
            serde_json::to_vec(&json!({"registry":registry(), "api_version":API_VERSION,
            "limits":LIMITS,"envelopes":contract_schema()}))
        {
            hasher.update(&bytes);
        }
        hasher.finalize().to_hex().to_string()
    })
}

/// Close fixed objects but retain typed dictionaries and local generated refs.
/// The wire integer policy is applied identically to every nested schema.
pub(crate) fn close_schema(value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        if object.contains_key("properties") {
            object.insert("additionalProperties".into(), Value::Bool(false));
        }
        let integer = object.get("type").is_some_and(|t| {
            t == "integer"
                || t.as_array()
                    .is_some_and(|a| a.iter().any(|x| x == "integer"))
        });
        if integer {
            let min = object
                .get("minimum")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MIN)
                .max(-MAX_SAFE_INTEGER);
            let max = object
                .get("maximum")
                .and_then(Value::as_u64)
                .unwrap_or(u64::MAX)
                .min(MAX_SAFE_INTEGER as u64);
            object.insert("minimum".into(), json!(min));
            object.insert("maximum".into(), json!(max));
        }
        for v in object.values_mut() {
            close_schema(v);
        }
    } else if let Some(array) = value.as_array_mut() {
        for v in array {
            close_schema(v);
        }
    }
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
        crate::edit::check_budget(&command)?;
    }
    Ok(command)
}

impl Session {
    pub(crate) fn control_mark(&self) -> (u64, VecDeque<std::sync::Arc<Event>>) {
        (self.control.sequence, self.control.events.clone())
    }
    pub(crate) fn restore_control_mark(&mut self, mark: (u64, VecDeque<std::sync::Arc<Event>>)) {
        self.control.sequence = mark.0;
        self.control.events = mark.1;
    }
    pub fn revision(&self) -> Revision {
        (*self.revision).clone()
    }
    pub fn session_id(&self) -> &str {
        &self.identity.0
    }
    pub(crate) fn invalidate_revision(&mut self, committed: bool) -> Result<()> {
        self.generation = self.generation.checked_add(1).ok_or(EditError::Limit)?;
        self.revision =
            std::sync::Arc::new(Revision(format!("{}:{}", self.identity.0, self.generation)));
        self.control.push(
            &self.identity.0,
            self.revision(),
            if committed {
                Payload::ProjectChanged {
                    full_snapshot_required: true,
                }
            } else {
                Payload::RevisionChanged
            },
        );
        Ok(())
    }
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
    fn admit_and_run(
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
        let fingerprint = blake3::hash(&serde_json::to_vec(&json!({"command":request.command,"params":request.params,
            "base_rev":request.base_rev,"api_version":request.api_version,"registry_hash":request.registry_hash,"session_id":request.session_id})).map_err(|_| EditError::InvalidParams)?);
        if let Command::Export { path, .. }
        | Command::ExportSubtitles { path, .. }
        | Command::BurnSubtitles { path, .. } = &command
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
            if let Some(receipt) = self.control.receipts.get(&key) {
                if receipt.fingerprint != fingerprint {
                    return Err(EditError::IdempotencyConflict);
                }
                let mut response = receipt.response.clone();
                response.request_id.clone_from(&request.request_id);
                return Ok(response);
            }
            if self.control.receipts.len() >= MAX_RECEIPTS
                || self
                    .control
                    .receipt_bytes
                    .saturating_add(RECEIPT_RESERVATION)
                    > MAX_RECEIPT_BYTES
            {
                return Err(EditError::ReceiptCapacity);
            }
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
        let value = match self.run_control(command, runner, fingerprint) {
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
            let bytes = serde_json::to_vec(&response)
                .map_err(|_| EditError::InvalidParams)?
                .len()
                + key.0.len()
                + key.1.len()
                + 32;
            self.control.receipt_bytes += bytes;
            self.control.receipts.insert(
                key,
                Receipt {
                    fingerprint,
                    response: response.clone(),
                },
            );
        }
        match &response.value {
            ResponseValue::Result(result) => {
                // Large read results use the response path; event consumers resync.
                if matches!(
                    result,
                    ControlResult::Edit { .. }
                        | ControlResult::JobAccepted { .. }
                        | ControlResult::CancelRequested { .. }
                ) {
                    self.control.push(
                        &self.identity.0,
                        self.revision(),
                        Payload::CommandResult {
                            request_id: request.request_id.clone(),
                            result: result.clone(),
                        },
                    );
                }
            }
            ResponseValue::Error(code) => self.control.push(
                &self.identity.0,
                self.revision(),
                Payload::CommandError {
                    request_id: request.request_id.clone(),
                    code: *code,
                },
            ),
        }
        Ok(response)
    }
    fn run_control(
        &mut self,
        command: Command,
        runner: &mut JobRunner,
        fingerprint: blake3::Hash,
    ) -> Result<ControlResult> {
        match command {
            Command::Inspect {} => {
                if self.transaction_active() {
                    return Err(EditError::TransactionActive);
                }
                let bytes = bounded_json(self.project().data(), MAX_RESPONSE_BYTES - 4096)?;
                let value: Value =
                    serde_json::from_slice(&bytes).map_err(|_| EditError::InvalidParams)?;
                check_integers(&value)?;
                Ok(ControlResult::Inspection {
                    project: self.project().data().clone(),
                })
            }
            Command::Catalog {} => {
                bounded_json(registry(), MAX_RESPONSE_BYTES - 4096)?;
                Ok(ControlResult::Catalog {
                    entries: registry().to_vec(),
                })
            }
            Command::ProviderCatalog {} => Ok(ControlResult::Providers {
                entries: crate::subtitles::provider_options()
                    .into_iter()
                    .map(ProviderView::from)
                    .collect(),
            }),
            Command::JobStatus { job } => {
                let mut snapshot =
                    JobView::from(runner.snapshot(job).ok_or(EditError::JobNotRetained)?);
                if snapshot.kind == JobKind::Preview
                    && snapshot.state == JobState::Completed
                    && runner.output_bytes(job).is_none()
                {
                    return Err(EditError::JobNotRetained);
                }
                if snapshot.kind == JobKind::Preview {
                    snapshot.result = runner
                        .output_bytes(job)
                        .map(serde_json::from_slice)
                        .transpose()
                        .map_err(|_| EditError::InvalidParams)?;
                }
                Ok(ControlResult::Job { snapshot })
            }
            Command::JobCancel { job } => {
                runner.cancel_job(job)?;
                Ok(ControlResult::CancelRequested { job })
            }
            Command::Preview { frames } => crate::control_preview::preview(self, frames, runner),
            Command::Export {
                range,
                path,
                overwrite,
            } => {
                if self.transaction_active() {
                    return Err(EditError::TransactionActive);
                }
                let mut settings = render::ExportSettings::from_project(self.project());
                settings.overwrite = if overwrite {
                    project::atomic::Overwrite::Replace
                } else {
                    project::atomic::Overwrite::Refuse
                };
                let (job, _) = crate::submit_export(
                    runner,
                    control_key(fingerprint)?,
                    self.project().clone(),
                    range,
                    settings,
                    path.clone(),
                )?;
                Ok(ControlResult::JobAccepted {
                    job,
                    revision: self.revision(),
                    paths: vec![path],
                })
            }
            command @ (Command::ExportSubtitles { .. } | Command::BurnSubtitles { .. }) => {
                let path = match &command {
                    Command::ExportSubtitles { path, .. } | Command::BurnSubtitles { path, .. } => {
                        path.clone()
                    }
                    _ => return Err(EditError::InvalidParams),
                };
                let (job, _) =
                    self.submit_export_command(runner, control_key(fingerprint)?, command)?;
                Ok(ControlResult::JobAccepted {
                    job,
                    revision: self.revision(),
                    paths: vec![path],
                })
            }
            command => self
                .apply(command)
                .map(|outcome| ControlResult::Edit { outcome }),
        }
    }
    /// Atomically capture committed state and a listener watermark under Session
    /// ownership. Preview state is deliberately not presented as committed state.
    pub fn snapshot_and_subscribe(&mut self, runner: &JobRunner) -> Result<Snapshot> {
        if self.transaction_active() {
            return Err(EditError::TransactionActive);
        }
        self.observe_jobs(runner);
        Ok(Snapshot {
            project: self.project().clone(),
            revision: self.revision(),
            jobs: runner.snapshots().into_iter().map(JobView::from).collect(),
            cursor: Cursor {
                session: self.session_id().into(),
                seq: self.control.sequence,
            },
        })
    }
    /// Pump from the same owning loop as JobRunner::tick. Does not drain another
    /// consumer's runner events or cancel work when a listener disappears.
    pub fn observe_jobs(&mut self, runner: &JobRunner) {
        let snapshots = runner.snapshots();
        self.control
            .jobs
            .retain(|id, _| snapshots.iter().any(|s| s.id == *id));
        for snapshot in snapshots {
            let view = JobView::from(snapshot);
            let changed = self.control.jobs.get(&view.id).is_none_or(|old| {
                old.state != view.state
                    || old.stage != view.stage
                    || old.attempts != view.attempts
                    || old.progress != view.progress
            });
            if changed {
                self.control.jobs.insert(view.id, view.clone());
                self.control.push(
                    &self.identity.0,
                    self.revision(),
                    Payload::JobChanged { snapshot: view },
                );
            }
        }
    }
    pub fn events(&self, cursor: &mut Cursor) -> Result<Vec<Event>> {
        if cursor.session != self.session_id()
            || cursor.seq > self.control.sequence
            || self.control.events.front().is_some_and(|e| {
                e.seq
                    .parse::<u64>()
                    .is_ok_and(|seq| cursor.seq.saturating_add(1) < seq)
            })
        {
            return Err(EditError::ResyncRequired);
        }
        let events = self
            .control
            .events
            .iter()
            .filter(|e| e.seq.parse::<u64>().is_ok_and(|seq| seq > cursor.seq))
            .map(|e| (**e).clone())
            .collect();
        cursor.seq = self.control.sequence;
        Ok(events)
    }
}
pub(crate) fn control_key(fingerprint: blake3::Hash) -> Result<crate::cache::CacheKey> {
    crate::cache::CacheKey::new(crate::cache::KeyInputs {
        kind: crate::cache::ArtifactKind::Preview,
        source: [0; 32],
        provider: "control",
        model: "command",
        version: "1",
        settings: fingerprint.as_bytes(),
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
}
struct Capped {
    bytes: Vec<u8>,
    max: usize,
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
struct JsonSeed<'a> {
    depth: usize,
    nodes: &'a mut usize,
    text: &'a mut usize,
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

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        reason = "injected capacity and generation exhaustion"
    )]
    use super::*;
    struct Allow;
    impl Policy for Allow {
        fn authorize(&self, _: &Command, _: &project::Project) -> Result<()> {
            Ok(())
        }
    }
    #[test]
    fn byte_capacity_is_reserved_before_mutation_and_generation_exhaustion_is_atomic() {
        let mut session = Session::new(
            project::Project::empty(project::ProjectId(0), "before".into()).unwrap(),
            10,
        )
        .unwrap();
        let mut runner = JobRunner::new(
            crate::jobs::RunnerConfig {
                concurrency: [1, 1, 1],
                queue: 1,
                retained: 1,
                network_attempts: 1,
                backoff: std::time::Duration::from_millis(1),
                progress_interval: std::time::Duration::from_millis(10),
            },
            crate::ByteBudget::new(1 << 20).unwrap(),
            std::sync::Arc::new(crate::jobs::SystemClock::default()),
        )
        .unwrap();
        let before = session.project().clone();
        session.control.receipt_bytes = MAX_RECEIPT_BYTES - RECEIPT_RESERVATION + 1;
        let request = Request {
            api_version: API_VERSION,
            registry_hash: Some(registry_hash().into()),
            session_id: session.session_id().into(),
            request_id: "capacity".into(),
            idempotency_key: Some("new".into()),
            base_rev: Some(session.revision()),
            command: "project.rename".into(),
            params: json!({"name":"never"}),
        };
        let response = session
            .control_request(
                request,
                Caller {
                    principal: "test",
                    external: true,
                    policy: &Allow,
                },
                &mut runner,
            )
            .unwrap();
        assert!(matches!(
            response.value,
            ResponseValue::Error(ErrorCode::ReceiptCapacity)
        ));
        assert_eq!(session.project(), &before);
        assert_eq!(session.history_len(), (0, 0));
        session.generation = u64::MAX;
        assert!(matches!(
            session.apply(Command::RenameProject {
                name: "never".into()
            }),
            Err(EditError::Limit)
        ));
        assert_eq!(session.project(), &before);
        session.generation = u64::MAX - 2;
        let result = session.transaction(|s| {
            s.apply(Command::RenameProject {
                name: "preview".into(),
            })?;
            s.apply(Command::RenameProject {
                name: "never".into(),
            })
        });
        assert!(matches!(result, Err(EditError::Limit)));
        assert_eq!(session.project(), &before);
        assert_eq!(session.history_len(), (0, 0));
        assert_eq!(session.generation, u64::MAX);
    }
}
