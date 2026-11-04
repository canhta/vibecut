//! Headless project commands and bounded, atomic snapshot history.
mod budget;
pub mod cache;
mod commands;
pub mod jobs;
pub use budget::{BudgetSnapshot, ByteBudget, ByteLease, ByteOwner};
mod edit;
mod session;
pub use commands::{
    Command, CommandSpec, CreatedId, FieldSchema, Move, NewItem, Outcome, registry,
};
pub use session::{JobTarget, Session};

/// Safe diagnostic categories: never include parameter values or user text.
#[derive(Debug, thiserror::Error)]
pub enum EditError {
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
