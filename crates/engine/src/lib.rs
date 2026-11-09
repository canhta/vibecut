//! Headless project commands with bounded execution resources.
pub mod control;
mod session;
pub(crate) use session::transitions;
pub use session::{
    Availability, Command, CommandKind, CommandSpec, CreatedId, DEFAULT_BOOKMARK_COLOR, Effect,
    Example, Fixture, GrantClass, ItemParamPatch, ItemPlacement, KeyframeRef, Lifecycle, Move,
    NewItem, Outcome, Placement, TrackPlacement, TrimEdge, lifecycle, registry,
};
pub use session::{FontError, JobTarget, Session, SoundError};
mod runtime;
pub use runtime::cache;
pub use runtime::jobs;
pub(crate) use runtime::publication;
pub use runtime::transfer;
pub use runtime::{BudgetSnapshot, ByteBudget, ByteLease, ByteOwner};
pub use runtime::{
    submit_export, submit_export_publication, submit_export_with_sources,
    submit_scene_exports_with_sources, subtitle_file,
};
mod store;
pub use store::library;
pub use store::recovery;
pub mod subtitles;
pub mod transcript;
pub use transcript::translation;
mod analysis;
pub use analysis::loudness;
pub use analysis::scene_detection;
pub use analysis::speech_activity;
pub use analysis::{
    AnalysisJob, AnalysisRequest, AnalysisResult, AnalysisSettings, FaceTrack, Scene,
    SecondAnalysis, ShotAnalysis, SpeakerRef, SpeakerState,
};
mod artifacts;
pub(crate) use artifacts::control_preview;
pub use artifacts::effect_thumbnails;
pub use artifacts::snapshot;
pub use artifacts::thumbnails;
pub mod providers;
pub use ai::clips::{
    ClipDropReason, ClipProposal, ClipRole, ClipSettings, ClipSource, ClipSourceSegment,
    DroppedClip,
};
pub use captions::{AssWarning, AssWarningKind, Format, NfcText};
pub use providers::models;

pub use subtitles::{AssImportReport, AssImportRequest, prepare_ass_import};
/// One media asset's timeline usage, retained in the typed removal refusal.
/// Item IDs are stable identifiers, never user-authored text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetUsage {
    pub asset: media::AssetId,
    pub items: Vec<project::ItemId>,
}

/// Typed reason payload for a clip-boundary refusal.
/// Owned with `EditError` so feature workflows can depend on the error contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryFailure {
    NoSafeFrame,
    SegmentDuration,
    TotalDuration,
    SegmentOverlap,
    Candidate,
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
        reason: BoundaryFailure,
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
    Sounds(#[from] SoundError),
    #[error("font catalog operation failed")]
    Font(#[from] FontError),
    #[error("local model {0} is not installed; run `vibecut-cli model download {0}`")]
    ModelMissing(&'static str),
    #[error("local Gipformer runner is not installed")]
    RunnerMissing,
    #[error("provider {0} requires its API key")]
    CredentialMissing(&'static str),
    #[error("auto language detection requires the local whisper.cpp provider")]
    AutoLanguageUnsupported,
    #[error("no speech recognized")]
    NoSpeech,
    #[error("speech recognition confidence is too low")]
    LowConfidence,
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

pub use runtime::{default_file_name, sanitize_file_name};
