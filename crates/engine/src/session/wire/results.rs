//! Defines the typed results a control command returns.
use super::{
    BrandKitView, EmphasisSuggestion, ErrorCode, FillerCandidate, FontEntry, FontLoad, JobId,
    JobKind, JobStage, JobState, MusicPage, PauseCandidates, PreviewResult, Revision,
    SavedMusicEntry, SavedSoundEntry, SnapshotResult, SoundPage, StickerPage,
};
use crate::{CommandSpec, Outcome};
use serde::Serialize;

#[derive(Clone, Serialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlResult {
    Clipboard {
        payload: Option<crate::KeyframeClipboard>,
    },
    Edit {
        outcome: Outcome,
    },
    Inspection {
        project: Box<project::ProjectData>,
    },
    /// `output_schema` is the schema of this enum, shared by every entry.
    Catalog {
        output_schema: serde_json::Value,
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
    /// The active brand kit, or `None` when the store has no active kit.
    BrandKit {
        kit: Option<BrandKitView>,
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
    StickerSearch {
        page: StickerPage,
    },
    MusicSearch {
        page: MusicPage,
    },
    MusicSaved {
        saved: SavedMusicEntry,
    },
    MusicUnsaved {
        id: String,
        removed: bool,
    },
    Fillers {
        candidates: Vec<FillerCandidate>,
    },
    ChapterSidecars {
        paths: super::ChapterSidecars,
    },
    JobAccepted {
        word_highlight_warnings: project::CaptionWordWarnings,
        job: JobId,
        revision: Revision,
        paths: Vec<std::path::PathBuf>,
    },
    /// `clips.export` accepted: one job per selected short, in selection order. `shots` is
    /// `None` when the per-shot analysis laid the shorts out, else why every short is a
    /// centred Fill.
    ClipExports {
        revision: Revision,
        shorts: Vec<super::ClipExportShort>,
        shots: Option<super::ClipShotsDiagnostic>,
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
    /// A project copy, bundle or extracted project was published.
    Published {
        /// The copy's project file, the bundle directory or the extracted project file.
        path: std::path::PathBuf,
        /// Identity of the published project, as a decimal string.
        project_id: String,
        /// Distinct media files copied, or present in the extracted bundle.
        copied: usize,
        /// Assets with no media; they enter the relink state.
        missing: Vec<u64>,
        /// Hex ids of LUTs with no copied bundle.
        missing_luts: Vec<String>,
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
            Self::Clipboard { .. } => "clipboard",
            Self::Edit { .. } => "edit",
            Self::Inspection { .. } => "inspection",
            Self::Catalog { .. } => "catalog",
            Self::Providers { .. } => "providers",
            Self::Fonts { .. } => "fonts",
            Self::Font { .. } => "font",
            Self::BrandKit { .. } => "brand_kit",
            Self::SoundSearch { .. } => "sound_search",
            Self::SoundSaved { .. } => "sound_saved",
            Self::SoundUnsaved { .. } => "sound_unsaved",
            Self::StickerSearch { .. } => "sticker_search",
            Self::MusicSearch { .. } => "music_search",
            Self::MusicSaved { .. } => "music_saved",
            Self::MusicUnsaved { .. } => "music_unsaved",
            Self::Fillers { .. } => "fillers",
            Self::ChapterSidecars { .. } => "chapter_sidecars",
            Self::JobAccepted { .. } => "job_accepted",
            Self::ClipExports { .. } => "clip_exports",
            Self::Job { .. } => "job",
            Self::Snapshot { .. } => "snapshot",
            Self::CancelRequested { .. } => "cancel_requested",
            Self::Published { .. } => "published",
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
    /// Durable descriptor for a subsequent color.register_lut edit.
    Lut(project::LutRecord),
    Captions(Box<super::CaptionsResult>),
    Translations(Box<super::TranslationsResult>),
    /// Inline JPEG frames of a `preview` job.
    Preview(PreviewResult),
    /// Reviewed-span candidates of `timeline.pause_candidates`.
    Pauses(PauseCandidates),
    /// The chapter document of `chapters.generate`.
    Chapters(Box<ai::text::chapters::ChapterDocument>),
    /// Classified occurrences of `fillers.classify`.
    Fillers {
        candidates: Vec<FillerCandidate>,
    },
    /// Model-chosen emphasis words of `captions.suggest_emphasis`.
    Emphasis {
        suggestions: Vec<EmphasisSuggestion>,
    },
    /// Measured values and the gains `audio.apply_loudness` takes.
    Loudness(super::LoudnessResult),
    /// Source-time candidates of `media.detect_scenes`.
    Scenes(super::DetectionResult),
    /// Per-shot report of `reframe.generate`; `reframe.apply` reads the job's
    /// retained analysis instead of carrying it here.
    Reframe(super::ReframeReport),
    /// Ranked candidates and source-timed cues of `clips.suggest`; `clips.export` reads
    /// the job's retained bytes instead of carrying them back.
    Clips(Box<super::ClipsResult>),
}
