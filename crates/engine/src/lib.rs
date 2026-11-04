//! Headless project commands and bounded, atomic snapshot history.
mod budget;
pub mod cache;
mod commands;
pub mod jobs;
pub mod models;
pub mod recovery;
pub mod translation;
pub use budget::{BudgetSnapshot, ByteBudget, ByteLease, ByteOwner};
mod edit;
mod rendering;
mod session;
pub use commands::{
    Command, CommandSpec, CreatedId, FieldSchema, Move, NewItem, Outcome, registry,
};
pub use rendering::{submit_export, subtitle_file};
pub use session::{JobTarget, Session};

/// Safe diagnostic categories: never include parameter values or user text.
#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("cache is in use")]
    CacheBusy,
    #[error("model transfer response does not match the requested representation")]
    ModelResponse,
    #[error("model storage is full")]
    StorageFull,

    #[error("recovery candidate is corrupt or incomplete")]
    RecoveryCorrupt,
    #[error("persistence operation is already active")]
    RecoveryBusy,
    #[error("recovered or never-saved work requires save-as")]
    SaveAsRequired,
    #[error("session persistence failed")]
    PersistenceFailed(#[source] std::sync::Arc<EditError>),
    #[error("job queue is full")]
    QueueFull,
    #[error("aggregate byte budget is full")]
    BudgetFull,
    #[error("artifact exceeds aggregate budget")]
    ArtifactTooLarge,
    #[error("job cancelled")]
    Cancelled,
    #[error("job worker unwound")]
    WorkerPanicked,
    #[error("job state lock poisoned")]
    JobPoisoned,
    #[error("transient network failure")]
    NetworkTransient,
    #[error("job result is stale")]
    StaleResult,
    #[error("corrupt cache entry skipped")]
    CacheCorrupt,
    #[error("render execution failed")]
    Render(#[from] render::Error),
    #[error("AI execution failed")]
    Ai(#[from] ai::Error),
    #[error("job/cache IO failed")]
    Io(#[from] std::io::Error),
    #[error("unknown command")]
    UnknownCommand,
    #[error("invalid command parameters")]
    InvalidParams,
    #[error("command limit exceeded")]
    Limit,
    #[error("missing track {0:?}")]
    NoTrack(project::TrackId),
    #[error("missing item {0:?}")]
    NoItem(project::ItemId),
    #[error("track {0:?} is locked")]
    Locked(project::TrackId),
    #[error("items overlap on track {0:?}")]
    Overlap(project::TrackId),
    #[error("invalid edit: {0}")]
    Invalid(&'static str),
    #[error("unsupported edit: {0}")]
    Unsupported(&'static str),
    #[error("command unwound; transaction rolled back")]
    Panicked,
    #[error("a gesture or transaction is active")]
    TransactionActive,
    #[error("no gesture is active")]
    NoGesture,
    #[error("no history step available")]
    NoHistory,
    #[error("identifier space exhausted")]
    IdExhausted,
    #[error("project candidate rejected")]
    Project(#[from] project::Error),
    #[error("invalid edit time")]
    Time(#[from] time::TimeError),
    #[error("invalid caption mapping")]
    Captions(#[from] captions::Error),
}
pub type Result<T> = std::result::Result<T, EditError>;
