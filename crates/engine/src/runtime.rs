//! Executes bounded background work over shared resources.
mod budget;
pub use budget::{BudgetSnapshot, ByteBudget, ByteLease, ByteOwner};
mod cache_storage;
pub mod cache {
    //! Retains content-addressed execution artifacts.
    pub use super::cache_storage::{
        ArtifactCache, ArtifactKind, ArtifactLease, CacheKey, KeyInputs, ModelLease,
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
mod rendering;
pub use rendering::{
    submit_export, submit_export_publication, submit_export_with_sources, subtitle_file,
};
