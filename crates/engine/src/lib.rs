//! Headless project commands with bounded execution resources.
#![recursion_limit = "256"]
pub mod control;
mod session;
pub(crate) use session::transitions;
pub use session::wire::{Failure, JobFailurePolicy};
pub use session::{
    AssetUsage, BrandKitError, EditError, FontError, JobTarget, MusicError, Session, SessionError,
    SoundError,
};
pub use session::{
    Availability, CaptionPreset, CaptionStyleApply, ChapterSidecars, Command, CommandKind,
    CommandSpec, CreatedId, CueId, DEFAULT_BOOKMARK_COLOR, Effect, EmphasisMode, Example, Fixture,
    GrantClass, ItemClipboard, ItemParamPatch, ItemPasteSource, ItemPlacement, KeyframeClipboard,
    KeyframePasteSource, KeyframeRef, Lifecycle, Move, NewItem, Outcome, Placement,
    TextAnimationPreset, TrackPlacement, TranscriptCue, TranscriptView, TranscriptWord, TrimEdge,
    WordId, declaration_order_error, lifecycle, registry,
};
mod runtime;
pub use runtime::cache;
pub use runtime::jobs;
pub(crate) use runtime::publication;
pub use runtime::transfer;
pub use runtime::{
    AssetExport, plan_asset_exports, submit_asset_exports, submit_export,
    submit_export_publication, submit_export_with_sources, submit_scene_exports_with_sources,
    subtitle_file,
};
pub use runtime::{
    BudgetSnapshot, ByteBudget, ByteBudgetHooks, ByteLease, ByteOwner, Error as RuntimeError,
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
pub use analysis::emphasis;
pub use analysis::fillers;
pub use analysis::loudness;
pub use analysis::pause_candidates;
pub use analysis::scene_detection;
pub use analysis::speech_activity;
pub use analysis::waveform;
pub use analysis::{
    AnalysisJob, AnalysisRequest, AnalysisResult, AnalysisSettings, FaceTrack, ReframeAnalysis,
    ReframeJob, ReframeKeys, ReframeReason, ReframeRequest, ReframeSample, ReframeShot,
    ReframeTarget, Scene, SecondAnalysis, ShotAnalysis, ShotKeys, SpeakerRef, SpeakerState,
    emit_reframe,
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

pub use runtime::{default_file_name, sanitize_file_name};
pub use subtitles::{AssImportReport, AssImportRequest, prepare_ass_import};

pub use artifacts::Error as ArtifactError;

pub use analysis::Error as AnalysisError;
