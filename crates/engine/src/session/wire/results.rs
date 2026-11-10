//! Defines the typed results a control command returns.
use super::{
    ErrorCode, FillerCandidate, FontEntry, FontLoad, JobId, JobKind, JobStage, JobState,
    PauseCandidates, PreviewResult, Revision, SnapshotResult,
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
    pub result: Option<PreviewResult>,
    /// Pause candidates, present on a completed `analyze_pauses` job.
    pub pauses: Option<PauseCandidates>,
    /// Chapter document, present on a completed `generate_chapters` job.
    pub chapters: Option<Box<ai::chapters::ChapterDocument>>,
}
