//! Defines the transport-neutral wire contract of one session.
mod clips;
mod contract;
mod envelope;
mod json;
mod ledger;
mod limits;
mod results;
mod views;

pub use clips::{
    ClipAdjustment, ClipCandidateView, ClipEdge, ClipEnergy, ClipExportShort, ClipPieceRange,
    ClipPieceSpan, ClipRefusal, ClipShotsDiagnostic, ClipSubtitleFile, ClipsResult,
};
pub use contract::{
    contract_schema, output_schema, reference_markdown, registry_hash, skill_reference_markdown,
};
pub use envelope::{
    API_VERSION, ControlError, ErrorCode, Event, Failure, JobFailurePolicy, Payload, Request,
    Response, ResponseValue, Revision,
};
pub use json::{bounded_json, parse_frame};
pub(crate) use json::{budget, close_schema, validate_wire};
pub use ledger::{Cursor, Snapshot};
pub(crate) use ledger::{Ledger, Receipt};
pub use limits::*;
pub use results::{ControlResult, JobOutput, JobView, ProviderView};
pub use views::{
    BrandColorView, BrandEntryState, BrandKitView, BrandMediaView, CaptionsResult, ChangeKind,
    ChapterSidecars, CostView, DetectionResult, EmphasisSuggestion, FillerCandidate, FontEntry,
    FontLoad, FontLoadState, FontOrigin, FontState, JobId, JobKind, JobStage, JobState,
    LoudnessResult, Measured, MusicEntry, MusicPage, MusicSource, OutputSpan, PauseCandidates,
    PreviewFrame, PreviewResult, ReframeReason, ReframeReport, ReframeShotReport, ReframeTarget,
    SavedMusicEntry, SavedSoundEntry, SceneCandidate, SnapshotResult, SoundEntry, SoundPage,
    SoundSource, StickerCategory, StickerEntry, StickerPage, TranslationsResult,
};
