//! Admits transport-neutral commands against committed session state.
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
    /// Trusted snapshot replacement grant; the request itself has no overwrite flag.
    /// Without a grant, staging and final publication both use atomic no-clobber.
    fn snapshot_overwrite(&self) -> bool {
        false
    }

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
    EmptyProject,
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
            EditError::EmptyProject => Self::EmptyProject,
            EditError::Panicked | EditError::WorkerPanicked | EditError::Io(_) => Self::Failed,
            EditError::Project(project::Error::Io(_)) => Self::Failed,
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
        project: Box<project::ProjectData>,
    },
    Catalog {
        entries: Vec<crate::CommandSpec>,
    },
    Providers {
        entries: Vec<ProviderView>,
    },
    Fonts {
        entries: Vec<crate::fonts::FontEntry>,
    },
    Font {
        font: crate::fonts::FontLoad,
    },
    JobAccepted {
        job: JobId,
        revision: Revision,
        paths: Vec<std::path::PathBuf>,
    },
    Job {
        snapshot: JobView,
    },
    Snapshot {
        snapshot: crate::snapshot::SnapshotResult,
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
            Self::Fonts { .. } => "fonts",
            Self::Font { .. } => "font",
            Self::JobAccepted { .. } => "job_accepted",
            Self::Job { .. } => "job",
            Self::Snapshot { .. } => "snapshot",
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
                crate::subtitles::CatalogOperation::SuggestClips => "suggest_clips",
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
struct Capped {
    bytes: Vec<u8>,
    max: usize,
}
struct JsonSeed<'a> {
    depth: usize,
    nodes: &'a mut usize,
    text: &'a mut usize,
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
mod resync;

mod contract;

mod admission;

mod receipts;

mod dispatch;

pub use contract::{contract_schema, reference_markdown, registry_hash};

pub(crate) use contract::close_schema;

pub(crate) use admission::{decode_command, validate_wire};

pub use admission::{bounded_json, parse_frame};

use receipts::control_fingerprint;

pub(crate) use dispatch::control_key;
