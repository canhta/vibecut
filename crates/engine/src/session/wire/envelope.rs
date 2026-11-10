//! Defines the versioned envelopes a transport exchanges with a session.
use super::{ControlResult, JobView};
use crate::EditError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const API_VERSION: u32 = 2;

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
            EditError::Permission => Self::Permission,
            // Each substrate type declares its own wire class; the kernel only delegates.
            EditError::Providers(error) => error.code(),
            EditError::Transcript(error) => error.code(),
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
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("api_version", &self.api_version)
            .finish_non_exhaustive()
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
