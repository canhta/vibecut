//! Headless project commands with bounded execution resources.
pub mod control;
mod session;
pub(crate) use session::transitions;
use session::wire::ErrorCode;
pub use session::wire::Failure;
pub use session::{
    Availability, CaptionPreset, CaptionStyleApply, ChapterSidecars, Command, CommandKind,
    CommandSpec, CreatedId, CueId, DEFAULT_BOOKMARK_COLOR, Effect, Example, Fixture, GrantClass,
    ItemParamPatch, ItemPlacement, KeyframeRef, Lifecycle, Move, NewItem, Outcome, Placement,
    TextAnimationPreset, TrackPlacement, TranscriptCue, TranscriptView, TranscriptWord, TrimEdge,
    WordId, declaration_order_error, lifecycle, registry,
};
pub use session::{FontError, JobTarget, Session, SoundError};
mod runtime;
pub use runtime::cache;
pub use runtime::jobs;
pub(crate) use runtime::publication;
pub use runtime::transfer;
pub use runtime::{
    BudgetSnapshot, ByteBudget, ByteBudgetHooks, ByteLease, ByteOwner, Error as RuntimeError,
};
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
pub use analysis::chapters;
pub use analysis::fillers;
pub use analysis::loudness;
pub use analysis::pause_candidates;
pub use analysis::scene_detection;
pub use analysis::speech_activity;
pub use analysis::waveform;
pub use analysis::{
    AnalysisJob, AnalysisRequest, AnalysisResult, AnalysisSettings, FaceTrack, Scene,
    SecondAnalysis, ShotAnalysis, SpeakerRef, SpeakerState,
};
mod artifacts;
pub use artifacts::chapter_sidecars;
pub(crate) use artifacts::control_preview;
pub use artifacts::effect_thumbnails;
pub use artifacts::snapshot;
pub use artifacts::thumbnail_strip;
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

    #[error("cache is in use")]
    CacheBusy,
    #[error("model transfer response does not match the requested representation")]
    ModelResponse,
    #[error("model storage is full")]
    StorageFull,

    #[error(transparent)]
    Recovery(#[from] recovery::Error),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error("job cancelled")]
    Cancelled,
    #[error("transient network failure")]
    NetworkTransient,
    #[error("job result is stale")]
    StaleResult,
    #[error("loudness gains are missing or measured for different audio; measure again")]
    LoudnessStale,
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
    #[error(transparent)]
    Providers(#[from] providers::Error),
    #[error(transparent)]
    Transcript(#[from] transcript::Error),
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
    #[error("transcript selection is in another scene")]
    SceneBoundary,
    #[error("transcript cue text is edited and protects its words")]
    Protected,
    #[error("transcript cues are not adjacent")]
    CuesNotAdjacent,
    #[error("transcript cues are on different tracks")]
    CuesAcrossTracks,
    #[error("transcript cues are in different items")]
    CuesAcrossItems,
    #[error("items overlap on track {0:?}")]
    Overlap(project::TrackId),
    #[error("Project is empty")]
    EmptyProject,
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

pub use runtime::{default_file_name, sanitize_file_name};

impl EditError {
    fn classification(&self) -> (ErrorCode, bool) {
        use ErrorCode as C;
        let code = match self {
            Self::SchemaMismatch => C::SchemaMismatch,
            Self::SessionGone => C::SessionGone,
            Self::StaleRevision => C::StaleRevision,
            Self::Permission => C::Permission,
            Self::IdempotencyConflict => C::IdempotencyConflict,
            Self::ReceiptCapacity => C::ReceiptCapacity,
            Self::ResyncRequired => C::ResyncRequired,
            Self::JobNotRetained => C::JobNotRetained,
            Self::UnknownCommand => C::UnknownCommand,
            Self::InvalidParams => C::InvalidParams,
            Self::Limit => C::Limit,
            Self::Cancelled => C::Cancelled,
            Self::TransactionActive | Self::NoGesture => C::TransactionActive,
            Self::EmptyProject => C::EmptyProject,
            Self::Panicked => C::Failed,
            Self::Runtime(inner) => return (inner.code(), inner.transient()),
            Self::StaleResult => C::StaleResult,
            Self::NetworkTransient => return (C::Job, true),
            Self::Recovery(inner) => inner.code(),
            Self::Sounds(inner) => inner.code(),
            Self::Font(inner) => inner.code(),
            Self::Providers(inner) => inner.code(),
            Self::Transcript(inner) => inner.code(),
            Self::Render(render::Error::Unavailable(_)) => C::Unavailable,
            Self::Render(_) => C::Render,
            Self::Media(_) => C::Media,
            Self::Ai(inner) => {
                return (
                    if matches!(inner, ai::Error::Chapters(_)) {
                        C::Refused
                    } else {
                        C::Ai
                    },
                    inner.is_transient(),
                );
            }
            Self::Io(_) | Self::Project(project::Error::Io(_)) => C::Io,
            Self::Project(project::Error::Busy) => C::Busy,
            Self::Project(project::Error::Conflict) => C::Conflict,
            Self::Project(project::Error::Media(_)) => C::Media,
            Self::ControlIdentity
            | Self::CacheBusy
            | Self::ModelResponse
            | Self::StorageFull
            | Self::LoudnessStale
            | Self::NoTrack(_)
            | Self::NoAsset(_)
            | Self::AssetsInUse(_)
            | Self::NoItem(_)
            | Self::Locked(_)
            | Self::SceneBoundary
            | Self::Protected
            | Self::CuesNotAdjacent
            | Self::CuesAcrossTracks
            | Self::CuesAcrossItems
            | Self::Overlap(_)
            | Self::Invalid(_)
            | Self::Unsupported(_)
            | Self::NoHistory
            | Self::IdExhausted
            | Self::Project(_)
            | Self::Time(_)
            | Self::Captions(_) => C::Refused,
        };
        (code, false)
    }
}
impl Failure for EditError {
    fn code(&self) -> ErrorCode {
        self.classification().0
    }
    fn transient(&self) -> bool {
        self.classification().1
    }
}
