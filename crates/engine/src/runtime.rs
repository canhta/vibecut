//! Executes bounded background work over shared resources.
mod budget;
pub use budget::{BudgetSnapshot, ByteBudget, ByteBudgetHooks, ByteLease, ByteOwner};
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
mod verified_transfer;
pub mod transfer {
    //! Fetches verified downloads for fonts and models.
    pub(crate) use super::verified_transfer::header;
    pub use super::verified_transfer::response;
}
mod loudness_admission;
pub(crate) use loudness_admission::VERSION as LOUDNESS_VERSION;
pub use loudness_admission::{admit as admit_loudness, content_key as loudness_content_key};
mod rendering;
pub use rendering::{
    submit_export, submit_export_publication, submit_export_with_sources,
    submit_scene_exports_with_sources, subtitle_file,
};

pub(crate) use staged_publication::snapshot_project_name;
pub use staged_publication::{default_file_name, sanitize_file_name};
