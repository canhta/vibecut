//! Exports accepted shorts as ordered source segments.
use crate::error::Error;
use captions::{CueTrack, Format, LanguageTag, NfcText};
use engine::cache::{ArtifactKind, CacheKey, KeyInputs};
use engine::jobs::{
    CancelToken, JobBytes, JobId, JobKind, JobRunner, JobSnapshot, JobSpec, JobStage, JobState,
    ResourceClass,
};
use engine::{Error as EngineError, subtitle_file};
use project::{
    CaptionParams, Item, ItemId, ItemParams, Project, ProjectData, Timeline, Track, TrackId,
    TrackKind, VideoParams,
};
use std::{
    collections::BTreeSet,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use time::{Tick, TimeRange};
use unicode_segmentation::UnicodeSegmentation;

/// Matches the AI clip contract's `max_segments` ceiling.
pub const MAX_CLIP_SEGMENTS: usize = 32;
/// Bounds eager preparation of derived projects and the submitted output set.
pub const MAX_BATCH_CLIPS: usize = 1000;
/// Bounds cue content cloned into all derived projects in one batch.
pub const MAX_BATCH_CUE_BYTES: u64 = 256 << 20;
/// Longest UTF-8 base name before suffixes and extensions.
const MAX_BASE_BYTES: usize = 200;

/// One accepted short: a title and its source pieces in play order.
#[derive(Clone, Debug)]
pub struct ClipShort {
    pub title: NfcText,
    /// Canonical AI clip pieces in play order. Export reads each piece's range.
    pub segments: Vec<ai::clips::ClipSourceSegment>,
    /// Optional normalized source crop applied to every piece.
    pub crop: Option<project::Crop>,
}
impl ClipShort {
    pub fn new(title: impl Into<NfcText>, segments: Vec<ai::clips::ClipSourceSegment>) -> Self {
        Self {
            title: title.into(),
            segments,
            crop: None,
        }
    }
    /// Preserve the accepted suggestion's play order and ranges; crop stays centered.
    pub fn from_suggestion(suggestion: &ai::clips::ClipSuggestion) -> Self {
        Self {
            title: suggestion.proposal.title.clone(),
            segments: suggestion.segments.clone(),
            crop: None,
        }
    }
    /// Apply one normalized crop rectangle to every piece in this short.
    pub fn with_crop(mut self, crop: Option<project::Crop>) -> Self {
        self.crop = crop;
        self
    }
}

/// Inputs shared by every short in one batch.
#[derive(Clone, Debug)]
pub struct ClipBatchRequest {
    /// Source media holding every short's pieces.
    pub asset: media::AssetId,
    /// Source-timed cues; mapped through each short's ordered pieces.
    pub cues: CueTrack,
    pub shorts: Vec<ClipShort>,
    /// Existing output directory.
    pub directory: PathBuf,
    /// None for no captions, or one/two distinct languages to burn and write.
    pub languages: Vec<LanguageTag>,
    /// Sidecar formats; empty writes no subtitle files.
    pub subtitle_formats: Vec<Format>,
    pub overwrite: bool,
    /// Explicit consent for initially missing/changed visual sources.
    pub allow_degraded: bool,
    /// Per-artifact output and staging ceiling; 1 byte..=1 TiB.
    pub max_output_bytes: u64,
}
impl ClipBatchRequest {
    pub fn new(
        asset: media::AssetId,
        shorts: Vec<ClipShort>,
        directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            asset,
            cues: CueTrack::default(),
            shorts,
            directory: directory.into(),
            languages: Vec::new(),
            subtitle_formats: Vec::new(),
            overwrite: false,
            allow_degraded: false,
            max_output_bytes: 64 << 30,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipSubtitlePath {
    pub format: Format,
    pub path: PathBuf,
}

/// Artifacts written for one short. Every path is a complete file once published.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipPaths {
    pub video: PathBuf,
    pub subtitles: Vec<ClipSubtitlePath>,
}

/// Artifacts this prepared short already published, retained across retries.
#[derive(Default)]
struct Published {
    video: bool,
    subtitles: Vec<bool>,
}
struct PreparedClip {
    project: Arc<Project>,
    settings: render::ExportSettings,
    paths: ClipPaths,
    caption_track: Option<TrackId>,
    languages: Vec<LanguageTag>,
    input: CacheKey,
    duration: Tick,
    sidecar_bytes: u64,
    overwrite: bool,
    max_output_bytes: u64,
    published: Arc<Mutex<Published>>,
    /// Immutable original snapshot; only its asset identities are read, to keep
    /// output admission protecting every original project source.
    sources: Arc<Project>,
}
struct ClipJob {
    id: JobId,
    cancel: CancelToken,
}
/// Prepared shorts and their submitted jobs. Preparation performs no IO and
/// submits nothing; `submit` adds one export job per short to the caller's runner.
pub struct ClipBatch {
    clips: Vec<PreparedClip>,
    jobs: Vec<Option<ClipJob>>,
}

impl ClipBatch {
    pub fn len(&self) -> usize {
        self.clips.len()
    }
}

impl ClipBatch {
    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }
}

impl ClipBatch {
    /// Derived snapshot for preview or inspection; the batch owns it.
    pub fn project(&self, index: usize) -> Option<&Project> {
        self.clips.get(index).map(|clip| clip.project.as_ref())
    }
}

impl ClipBatch {
    /// Export settings for the prepared short, including portrait geometry,
    /// caption languages and per-piece crop placements.
    pub fn settings(&self, index: usize) -> Option<&render::ExportSettings> {
        self.clips.get(index).map(|clip| &clip.settings)
    }
}

impl ClipBatch {
    pub fn paths(&self, index: usize) -> Option<&ClipPaths> {
        self.clips.get(index).map(|clip| &clip.paths)
    }
}

impl ClipBatch {
    pub fn duration(&self, index: usize) -> Option<Tick> {
        self.clips.get(index).map(|clip| clip.duration)
    }
}

impl ClipBatch {
    pub fn job_id(&self, index: usize) -> Option<JobId> {
        self.jobs
            .get(index)
            .and_then(Option::as_ref)
            .map(|job| job.id)
    }
}

impl ClipBatch {
    /// Latest runner snapshot, or None when never submitted or evicted.
    pub fn snapshot(&self, runner: &JobRunner, index: usize) -> Option<JobSnapshot> {
        runner.snapshot(self.job_id(index)?)
    }
}

struct Options<'a> {
    asset: &'a media::SourceAsset,
    cues: &'a CueTrack,
    languages: &'a [LanguageTag],
    formats: &'a [Format],
    directory: &'a Path,
    overwrite: bool,
    allow_degraded: bool,
    max_output_bytes: u64,
}

#[derive(Default)]
struct NameSet {
    used: BTreeSet<String>,
}
mod preparation;

mod submission;

mod naming;

mod publication;

use submission::job_key;

use naming::{extension, name_base};

use publication::{mark_subtitle, mark_video, reconcile_published};
