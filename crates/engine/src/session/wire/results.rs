//! Defines the typed results a control command returns.
use super::{
    ErrorCode, FillerCandidate, FontEntry, FontLoad, JobId, JobKind, JobStage, JobState,
    PauseCandidates, PreviewResult, Revision, SavedSoundEntry, SnapshotResult, SoundPage,
};
use crate::{CommandSpec, Outcome};
use serde::Serialize;

#[derive(Clone, Serialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlResult {
    Edit {
        outcome: Outcome,
    },
    Inspection {
        project: Box<project::ProjectData>,
    },
    Catalog {
        entries: Vec<CommandSpec>,
    },
    Providers {
        entries: Vec<ProviderView>,
    },
    Fonts {
        entries: Vec<FontEntry>,
    },
    Font {
        font: FontLoad,
    },
    SoundSaved {
        saved: SavedSoundEntry,
    },
    SoundSearch {
        page: SoundPage,
    },
    SoundUnsaved {
        id: u64,
        removed: bool,
    },
    Fillers {
        candidates: Vec<FillerCandidate>,
    },
    ChapterSidecars {
        paths: super::ChapterSidecars,
    },
    JobAccepted {
        job: JobId,
        revision: Revision,
        paths: Vec<std::path::PathBuf>,
    },
    Job {
        snapshot: JobView,
    },
    Snapshot {
        snapshot: SnapshotResult,
    },
    CancelRequested {
        job: JobId,
    },
}
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
pub struct ProviderView {
    pub operation: String,
    pub provider: String,
    pub models: Vec<String>,
    pub remote: bool,
    pub native_timing: bool,
    pub languages: Vec<String>,
    pub target_languages: Vec<String>,
}
impl std::fmt::Debug for ControlResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self {
            Self::Edit { .. } => "edit",
            Self::Inspection { .. } => "inspection",
            Self::Catalog { .. } => "catalog",
            Self::Providers { .. } => "providers",
            Self::Fonts { .. } => "fonts",
            Self::Font { .. } => "font",
            Self::SoundSearch { .. } => "sound_search",
            Self::SoundSaved { .. } => "sound_saved",
            Self::SoundUnsaved { .. } => "sound_unsaved",
            Self::Fillers { .. } => "fillers",
            Self::ChapterSidecars { .. } => "chapter_sidecars",
            Self::JobAccepted { .. } => "job_accepted",
            Self::Job { .. } => "job",
            Self::Snapshot { .. } => "snapshot",
            Self::CancelRequested { .. } => "cancel_requested",
        };
        f.debug_struct("ControlResult")
            .field("kind", &kind)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
pub struct JobView {
    pub id: JobId,
    pub kind: JobKind,
    pub stage: JobStage,
    pub progress: f32,
    pub state: JobState,
    pub attempts: u8,
    pub error: Option<ErrorCode>,
    /// What a completed job produced; `None` while it runs and for kinds that publish
    /// files or caches instead of returning data.
    pub output: Option<JobOutput>,
}

/// The data a completed job returns, one variant per job kind that has any.
/// `control::output::decode` matches every [`JobKind`], so a new kind does not
/// compile until it names its output or states it has none. A job never edits
/// the project: the caller sends an ordinary edit command with the `revision`
/// from `JobAccepted`.
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JobOutput {
    /// Inline JPEG frames of a `preview` job.
    Preview(PreviewResult),
    /// Reviewed-span candidates of `timeline.pause_candidates`.
    Pauses(PauseCandidates),
    /// The chapter document of `chapters.generate`.
    Chapters(Box<ai::chapters::ChapterDocument>),
    /// Classified occurrences of `fillers.classify`.
    Fillers { candidates: Vec<FillerCandidate> },
    /// Measured values and the gains `audio.apply_loudness` takes.
    Loudness(super::LoudnessResult),
    /// Source-time candidates of `media.detect_scenes`.
    Scenes(super::DetectionResult),
}
