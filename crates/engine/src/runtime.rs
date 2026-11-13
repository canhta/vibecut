//! Executes bounded background work over shared resources.
use crate::session::wire::{ErrorCode, Failure, JobFailurePolicy};

/// Runtime failures retain typed causes without exposing allocation or file contents.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("AI execution failed")]
    Transfer(#[source] ai::Error),
    #[error("render execution failed")]
    Render(#[from] render::Error),
    #[error("media operation failed")]
    Media(#[from] media::MediaError),
    #[error(transparent)]
    Edit(#[from] crate::session::EditError),
    #[error("AI execution failed")]
    Model(#[source] ai::Error),
    #[error("job is absent or no longer retained")]
    JobNotRetained,
    #[error("identifier space exhausted")]
    IdExhausted,
    #[error("job cancelled")]
    Cancelled,
    #[error("loudness gains are missing or measured for different audio; measure again")]
    LoudnessStale,
    #[error("cache is in use")]
    CacheBusy,
    #[error("model transfer response does not match the requested representation")]
    ModelResponse,
    #[error("model storage is full")]
    StorageFull,
    #[error("transient network failure")]
    NetworkTransient,
    #[error("corrupt cache entry skipped")]
    CacheCorrupt,
    #[error("job queue is full")]
    QueueFull,
    #[error("job worker unwound")]
    WorkerPanicked,
    #[error("aggregate byte budget is full")]
    BudgetFull,
    #[error("artifact exceeds aggregate budget")]
    ArtifactTooLarge,
    #[error("job state lock poisoned")]
    Poisoned,
    #[error("job/cache IO failed")]
    Io(#[from] std::io::Error),
    #[error("command limit exceeded")]
    Limit,
    #[error("invalid edit: {0}")]
    Invalid(&'static str),
    #[error("background job failed: {0}")]
    Failed(std::sync::Arc<dyn Failure + 'static>),
}
impl Failure for Error {
    fn job_policy(&self) -> JobFailurePolicy {
        JobFailurePolicy {
            terminal_cancelled: matches!(self, Self::Cancelled),
            retry_after_seconds: None,
        }
    }
    fn code(&self) -> ErrorCode {
        match self {
            Self::Transfer(error) => error.code(),
            Self::Render(error) => error.code(),
            Self::Media(error) => error.code(),
            Self::Edit(error) => error.code(),
            Self::Model(_) => ErrorCode::Ai,
            Self::Cancelled => ErrorCode::Cancelled,
            Self::JobNotRetained => ErrorCode::JobNotRetained,
            Self::QueueFull => ErrorCode::QueueFull,
            Self::WorkerPanicked => ErrorCode::WorkerFailure,
            Self::BudgetFull | Self::ArtifactTooLarge => ErrorCode::Budget,
            Self::NetworkTransient | Self::CacheCorrupt | Self::Poisoned => ErrorCode::Job,
            Self::Io(_) => ErrorCode::Io,
            Self::Limit => ErrorCode::Limit,
            Self::IdExhausted
            | Self::LoudnessStale
            | Self::CacheBusy
            | Self::ModelResponse
            | Self::StorageFull
            | Self::Invalid(_) => ErrorCode::Refused,
            Self::Failed(cause) => cause.code(),
        }
    }
    fn transient(&self) -> bool {
        matches!(self, Self::NetworkTransient)
    }
}
impl From<project::Error> for Error {
    fn from(error: project::Error) -> Self {
        Self::Edit(error.into())
    }
}
impl From<captions::Error> for Error {
    fn from(error: captions::Error) -> Self {
        Self::Edit(error.into())
    }
}
impl From<crate::session::SessionError> for Error {
    fn from(error: crate::session::SessionError) -> Self {
        Self::Edit(error.into())
    }
}
pub type Result<T> = std::result::Result<T, Error>;

mod budget;
pub use budget::{BudgetSnapshot, ByteBudget, ByteBudgetHooks, ByteLease, ByteOwner};
mod cache_location;
mod cache_storage;
pub mod cache {
    //! Retains content-addressed execution artifacts.
    pub use super::cache_location::CacheRoot;
    pub use super::cache_storage::{
        ArtifactCache, ArtifactKind, ArtifactLease, CacheKey, CacheMaintenance,
        CacheMaintenanceReport, CacheOperation, KeyInputs, ModelLease,
    };
    pub(crate) use super::cache_storage::{NativeEdit, StoredArtifact, strong_etag};
}
mod runner;
pub mod jobs {
    //! Schedules bounded blocking execution.
    pub use super::runner::{
        CancelToken, Clock, JobBytes, JobContext, JobId, JobKind, JobOutput, JobRunner,
        JobSnapshot, JobSpec, JobStage, JobState, ResourceClass, RunnerConfig, SystemClock,
    };
}
mod staged_publication;
pub(crate) mod publication {
    //! Publishes admitted output artifacts.
    pub(crate) use super::staged_publication::Publication;
}
mod verified_transfer;
pub mod transfer {
    //! Fetches verified downloads for fonts and models.
    pub(crate) use super::verified_transfer::header;
    pub use super::verified_transfer::response;
}
pub(crate) mod loudness_admission;
mod rendering;
pub use rendering::{
    AssetExport, clip_export_range, plan_asset_exports, submit_asset_exports,
    submit_asset_exports_with_sources, submit_clip_export, submit_export,
    submit_export_publication, submit_export_with_sources, submit_scene_exports_with_sources,
    subtitle_file,
};

pub(crate) use staged_publication::snapshot_project_name;
pub use staged_publication::{default_file_name, sanitize_file_name};
