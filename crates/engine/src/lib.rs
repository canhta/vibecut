//! Headless project commands with bounded execution resources.
#![recursion_limit = "256"]
pub mod control;
mod session;
pub(crate) use session::transitions;
use session::wire::ErrorCode;
pub use session::wire::{Failure, JobFailurePolicy};
pub use session::{AssetUsage, EditError, FontError, JobTarget, Session, SessionError, SoundError};
pub use session::{
    Availability, CaptionPreset, CaptionStyleApply, ChapterSidecars, Command, CommandKind,
    CommandSpec, CreatedId, CueId, DEFAULT_BOOKMARK_COLOR, Effect, Example, Fixture, GrantClass,
    ItemParamPatch, ItemPlacement, KeyframeClipboard, KeyframePasteSource, KeyframeRef, Lifecycle,
    Move, NewItem, Outcome, Placement, TextAnimationPreset, TrackPlacement, TranscriptCue,
    TranscriptView, TranscriptWord, TrimEdge, WordId, declaration_order_error, lifecycle, registry,
};
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
pub use store::luts;
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
/// Safe diagnostic categories: never include parameter values or user text.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Library(#[from] library::Error),
    #[error(transparent)]
    Edit(#[from] EditError),
    #[error(transparent)]
    Control(#[from] control::ControlError),

    #[error(transparent)]
    Recovery(#[from] recovery::Error),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error(transparent)]
    Session(#[from] SessionError),
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
    #[error("Project is empty")]
    EmptyProject,
}
pub type Result<T> = std::result::Result<T, Error>;

pub use runtime::{default_file_name, sanitize_file_name};

impl Error {
    fn classification(&self) -> (ErrorCode, bool) {
        use ErrorCode as C;
        let code = match self {
            Self::Library(inner) => return (inner.code(), inner.transient()),
            Self::EmptyProject => C::EmptyProject,
            Self::Runtime(inner) => return (inner.code(), inner.transient()),
            Self::Session(inner) => return (inner.code(), inner.transient()),
            Self::Control(inner) => return (inner.code(), inner.transient()),
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
            Self::Io(_) => C::Io,
            Self::Edit(inner) => return (inner.code(), inner.transient()),
        };
        (code, false)
    }
}
impl Failure for Error {
    fn job_policy(&self) -> JobFailurePolicy {
        match self {
            Self::Library(error) => error.job_policy(),
            Self::Edit(error) => error.job_policy(),
            Self::Runtime(error) => error.job_policy(),
            Self::Session(error) => error.job_policy(),
            Self::Control(error) => error.job_policy(),
            Self::Ai(ai::Error::RateLimit {
                retry_after_seconds,
            }) => JobFailurePolicy {
                retry_after_seconds: *retry_after_seconds,
                ..Default::default()
            },
            _ => JobFailurePolicy::default(),
        }
    }
    fn code(&self) -> ErrorCode {
        self.classification().0
    }
    fn transient(&self) -> bool {
        self.classification().1
    }
}

impl From<project::Error> for Error {
    fn from(error: project::Error) -> Self {
        Self::Edit(error.into())
    }
}

impl From<time::TimeError> for Error {
    fn from(error: time::TimeError) -> Self {
        Self::Edit(error.into())
    }
}

impl From<captions::Error> for Error {
    fn from(error: captions::Error) -> Self {
        Self::Edit(error.into())
    }
}
