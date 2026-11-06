//! Headless project commands and bounded, atomic snapshot history.
pub mod ass_import;
mod budget;
pub mod cache;
pub mod clip_boundaries;
mod clip_export;
pub mod clip_suggest;
mod commands;
pub mod control;
mod control_preview;
pub mod fonts;
pub mod jobs;
pub mod library;
pub mod loudness;
pub mod models;
pub mod providers;
pub mod recovery;
pub mod scene_detection;
pub mod shot_analysis;
pub mod snapshot;
pub mod sounds;
pub mod subtitles;
mod thumbnail;
pub mod thumbnails;
pub mod translation;
pub use budget::{BudgetSnapshot, ByteBudget, ByteLease, ByteOwner};
pub use clip_export::{
    ClipBatch, ClipBatchRequest, ClipPaths, ClipShort, ClipSubtitlePath, MAX_BATCH_CLIPS,
    MAX_BATCH_CUE_BYTES, MAX_CLIP_SEGMENTS,
};
pub use clip_suggest::{
    ClipCandidate, ClipExportOutcome, ClipExportPaths, ClipExportPrep, ClipExportRequest,
    ClipRequest, ClipSuggestionOutcome, RejectedClip, SnapSettings, SuggestionOption,
    SuggestionProvider, check_suggestion_credentials, choose_suggestion, suggestion_options,
};
// These AI-owned types appear in clip workflow signatures, so adapters use them
// through this crate instead of taking a second dependency on `ai`.
pub use ai::clips::{
    ClipDropReason, ClipProposal, ClipRole, ClipSettings, ClipSource, ClipSourceSegment,
    DroppedClip,
};
pub use ass_import::{AssImportReport, AssImportRequest, prepare_ass_import};
pub use captions::{AssWarning, AssWarningKind, Format, NfcText};
mod edit;
pub mod effect_thumbnails;
mod publication;
mod rendering;
mod session;
mod transitions;
pub use commands::{
    Command, CommandKind, CommandSpec, CreatedId, Effect, Example, Fixture, ItemParamPatch,
    ItemPlacement, Move, NewItem, Outcome, Placement, TrackPlacement, registry,
};
pub use rendering::{
    submit_export, submit_export_publication, submit_export_with_sources, subtitle_file,
};
pub use session::{JobTarget, Session};

/// One media asset's timeline usage, retained in the typed removal refusal.
/// Item IDs are stable identifiers, never user-authored text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetUsage {
    pub asset: media::AssetId,
    pub items: Vec<project::ItemId>,
}

/// Safe diagnostic categories: never include parameter values or user text.
#[derive(Debug, thiserror::Error)]
pub enum EditError {
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
    #[error("job is absent or no longer retained")]
    JobNotRetained,
    #[error("control identity unavailable")]
    ControlIdentity,

    #[error("clip boundary refused at short {short}, segment {segment}: {reason:?}")]
    ClipBoundary {
        short: usize,
        segment: usize,
        reason: clip_boundaries::BoundaryFailure,
    },
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
    #[error("loudness gains are missing or measured for different audio; measure again")]
    LoudnessStale,
    #[error("background job failed: {0}")]
    JobFailure(std::sync::Arc<EditError>),
    #[error("corrupt cache entry skipped")]
    CacheCorrupt,
    #[error("render execution failed")]
    Render(#[from] render::Error),
    #[error("media operation failed")]
    Media(#[from] media::MediaError),
    #[error("AI execution failed")]
    Ai(#[from] ai::Error),
    #[error("sound library failed: {0}")]
    Sounds(#[from] sounds::Error),
    #[error("font catalog operation failed")]
    Font(#[from] fonts::FontError),
    #[error("local model {0} is not installed; run `vibecut-cli model download {0}`")]
    ModelMissing(&'static str),
    #[error("local Gipformer runner is not installed")]
    RunnerMissing,
    #[error("provider {0} requires its API key")]
    CredentialMissing(&'static str),
    #[error("auto language detection requires the local whisper.cpp provider")]
    AutoLanguageUnsupported,
    #[error("remote upload requires explicit consent")]
    ConsentRequired,
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
    #[error("missing media asset {0:?}")]
    NoAsset(media::AssetId),
    #[error("media asset is used by timeline items: {0:?}")]
    AssetsInUse(Vec<AssetUsage>),
    #[error("missing item {0:?}")]
    NoItem(project::ItemId),
    #[error("track {0:?} is locked")]
    Locked(project::TrackId),
    #[error("items overlap on track {0:?}")]
    Overlap(project::TrackId),
    #[error("Project is empty")]
    EmptyProject,
    #[error("invalid edit: {0}")]
    Invalid(&'static str),
    #[error("clip export refused at short {short}: {reason}")]
    ClipExport { short: usize, reason: &'static str },
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
