//! Defines the versioned envelopes a transport exchanges with a session.
use super::{ControlResult, JobView};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const API_VERSION: u32 = 15;

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
    Unavailable,
    Setup,
    Auth,
    Busy,
    Conflict,
    Io,
    Media,
    Render,
    Ai,
    Job,
    Quota,
    PlanRequired,
    RateLimit,
    Network,
    AudioDevice,
    Sounds,
    Budget,
    StaleResult,
    WorkerFailure,
    SoundLimit,
    SoundCancelled,
    SoundIo,
}
/// Protocol refusals shared by admission, receipt replay and event cursors.
#[derive(Debug, thiserror::Error)]
pub enum ControlError {
    #[error("unknown command")]
    UnknownCommand,
    #[error("invalid command parameters")]
    InvalidParams,
    #[error("command limit exceeded")]
    Limit,
    #[error("control schema version or catalog differs")]
    SchemaMismatch,
    #[error("session epoch differs")]
    SessionGone,
    #[error("project revision differs")]
    StaleRevision,
    #[error("control permission refused")]
    Permission,
    #[error("idempotency key conflicts")]
    IdempotencyConflict,
    #[error("session receipt capacity exhausted")]
    ReceiptCapacity,
    #[error("event cursor requires a new snapshot")]
    ResyncRequired,
}
impl Failure for ControlError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::UnknownCommand => ErrorCode::UnknownCommand,
            Self::InvalidParams => ErrorCode::InvalidParams,
            Self::Limit => ErrorCode::Limit,
            Self::SchemaMismatch => ErrorCode::SchemaMismatch,
            Self::SessionGone => ErrorCode::SessionGone,
            Self::StaleRevision => ErrorCode::StaleRevision,
            Self::Permission => ErrorCode::Permission,
            Self::IdempotencyConflict => ErrorCode::IdempotencyConflict,
            Self::ReceiptCapacity => ErrorCode::ReceiptCapacity,
            Self::ResyncRequired => ErrorCode::ResyncRequired,
        }
    }
}

/// Scheduling origin retained by a direct producer, never inherited by semantic wrappers.
/// This policy is local execution metadata and is not serialized into wire envelopes.
#[derive(Clone, Copy, Default)]
pub struct JobFailurePolicy {
    pub terminal_cancelled: bool,
    pub retry_after_seconds: Option<u32>,
}
/// Owner-local classification shared by control transports and job scheduling.
/// Code delegation preserves the cause; transience defaults to no retry.
pub trait Failure: std::error::Error + Send + Sync {
    fn code(&self) -> ErrorCode;
    fn job_policy(&self) -> JobFailurePolicy {
        JobFailurePolicy::default()
    }
    fn transient(&self) -> bool {
        false
    }
}
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("api_version", &self.api_version)
            .finish_non_exhaustive()
    }
}
#[allow(
    clippy::large_enum_variant,
    reason = "wire responses stay a plain tag/content enum; boxing the control result would change every adapter's response type"
)]
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
