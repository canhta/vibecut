//! Defines the feature result views a control response carries.
use super::Revision;
use captions::NfcText;
use project::{ItemId, LoudnessGains};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use time::{SourceRange, Tick};

/// Where a family comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FontOrigin {
    /// Shipped inside the application with a licence record.
    Bundled,
    /// Installed in the operating system.
    System,
    /// Curated Google Fonts family, downloadable on explicit request.
    Google,
}

/// Whether a family can render now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FontState {
    Available,
    /// A catalog family whose files are not in the verified local cache.
    NotDownloaded,
}

/// One family in a listing.
#[derive(Clone, Debug, PartialEq, Serialize, schemars::JsonSchema)]
pub struct FontEntry {
    pub family: String,
    pub origin: FontOrigin,
    /// Style names such as `Regular` or `Bold Italic`, sorted by weight.
    pub styles: Vec<String>,
    pub state: FontState,
    /// SPDX licence id of bundled and Google families.
    pub license: Option<String>,
    /// Upstream source URL of bundled and Google families.
    pub source: Option<String>,
    /// Bytes `fonts.download` would fetch, for families that are not downloaded.
    pub download_bytes: Option<u64>,
}

/// Outcome of loading a family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FontLoadState {
    /// Faces are registered and layout resolves the family.
    Loaded,
    /// Known catalog family with no cached files; layout falls back to the default family.
    NotDownloaded,
    /// Cached files failed their pinned hash or size; they are not used until downloaded again.
    CacheInvalid,
    /// No bundled, system or catalog family has this name; layout falls back.
    Unknown,
}

/// Result of loading one font family: its state and the family layout will use.
#[derive(Clone, Debug, PartialEq, Serialize, schemars::JsonSchema)]
pub struct FontLoad {
    pub family: String,
    pub state: FontLoadState,
    pub origin: Option<FontOrigin>,
    /// The family layout will use: the request when loaded, otherwise the default family.
    pub effective_family: String,
    pub styles: Vec<String>,
}

/// Where a sound search page came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SoundSource {
    Remote,
    Local,
}

/// One sound search result; also the record `sounds.save` upserts. Attribution is
/// precomputed by the library so every surface and export shows the same text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SoundEntry {
    pub id: u64,
    pub name: String,
    pub author: String,
    pub licence: String,
    pub attribution: String,
    pub source_url: String,
    pub audio_url: String,
    pub duration_ms: u64,
    pub downloads: u64,
}

/// One bounded page of sound search results. `commercial_only` echoes the switch
/// the request asked for, since a provider payload does not carry it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SoundPage {
    pub source: SoundSource,
    pub count: u64,
    pub page: u32,
    pub page_size: u32,
    pub has_next: bool,
    pub commercial_only: bool,
    pub results: Vec<SoundEntry>,
}

/// The record a `sounds.save` upsert produced.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SavedSoundEntry {
    pub id: u64,
    pub saved_at_unix_ms: u64,
    pub attribution: String,
    pub licence: String,
}

/// Published snapshot receipt; the PNG itself stays in `SnapshotImage`.
#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
pub struct SnapshotResult {
    pub path: PathBuf,
    pub time: Tick,
    pub timecode: String,
    pub width: u32,
    pub height: u32,
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub struct JobId(pub u64);

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Transcribe,
    Translate,
    SuggestClips,
    GenerateChapters,
    ClassifyFillers,
    AnalyzeShots,
    DetectScenes,
    AnalyzeSpeechActivity,
    GenerateWaveform,
    AnalyzePauses,
    GenerateClipThumbnails,
    MeasureLoudness,
    Preview,
    EffectThumbnail,
    Export,
    Import,
    #[serde(rename = "import.probe")]
    Probe,
    #[serde(rename = "import.thumbnail")]
    Thumbnail,
    Persist,
    DownloadModel,
    SoundDownload,
    DownloadFont,
}
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum JobStage {
    Extract,
    Segment,
    Queued,
    Starting,
    Preparing,
    Analyzing,
    Transcribe,
    Align,
    Translate,
    SuggestClips,
    Assemble,
    Process,
    Probe,
    Thumbnail,
    Admit,
    Render,
    Mix,
    Encode,
    Finalize,
    Download,
    Verify,
    Publish,
    Backoff,
    Complete,
}
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Cancelling,
    Publishing,
    Backoff,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PreviewFrame {
    pub frame: i64,
    pub output_tick: time::Tick,
    pub clips: Vec<project::ItemId>,
    pub width: u32,
    pub height: u32,
    /// JPEG bytes, encoded at quality 80; transports choose their binary encoding.
    pub jpeg: Vec<u8>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct PreviewResult {
    pub revision: Revision,
    pub frames: Vec<PreviewFrame>,
}
impl std::fmt::Debug for PreviewFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewFrame")
            .field("frame", &self.frame)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// The published `.txt` and `.json` chapter sidecar paths.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct ChapterSidecars {
    pub text: PathBuf,
    pub json: PathBuf,
}

/// One reviewed output-time span. Candidate queries produce these; reviewed-span
/// removals consume them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutputSpan {
    pub start: Tick,
    pub end: Tick,
}

/// Output-time pause candidates after padding, word-guard and merge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PauseCandidates {
    pub spans: Vec<OutputSpan>,
    /// Distinct source ranges the analysis decoded; zero for an inaudible timeline.
    pub analyzed_sources: u64,
}

/// One visible lexicon filler occurrence in output time. `timeline.filler_candidates`
/// produces these; `timeline.remove_fillers` consumes their `id`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FillerCandidate {
    /// Stable occurrence identity to pass back to `timeline.remove_fillers`.
    pub id: NfcText,
    /// The caption item whose output occurrence this is.
    pub item: ItemId,
    /// The retained cue that owns the words.
    pub cue: NfcText,
    /// The visible output range the candidate words cover.
    pub range: OutputSpan,
    /// Whether the match is a filled pause or a discourse marker.
    pub kind: captions::FillerKind,
    /// Whether the cue's source text was edited, so removal needs confirmation.
    pub edited_cue: bool,
}

/// Integrated loudness and true peak of one measured signal; `None` is silence.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Measured {
    pub integrated_lufs: Option<f64>,
    pub true_peak_db: Option<f64>,
}
/// One finished measurement: gains bound to the measured content, plus the
/// numbers they were derived from (clips alone; master after clip gains).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoudnessResult {
    pub gains: LoudnessGains,
    pub clips: Vec<(ItemId, Measured)>,
    pub master: Option<Measured>,
}

/// Whether a scene candidate is a hard cut or a gradual fade.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    HardCut,
    Fade,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneCandidate {
    /// Exact decoded source PTS; fade markers use the nearest observed midpoint (earlier on ties).
    pub at: Tick,
    /// Normalized measured change, a heuristic score rather than a calibrated probability.
    pub confidence: f32,
    pub kind: ChangeKind,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DetectionResult {
    pub(crate) key: [u8; 32],
    pub(crate) source: [u8; 32],
    pub(crate) range: SourceRange,
    pub candidates: Vec<SceneCandidate>,
    pub decoded_frames: u64,
}

/// What a provider run cost or may cost, as the CLI and `job_status` disclose it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CostView {
    Free,
    UpperBound { currency: String, micros: u64 },
    Unknown,
}
/// The finished transcription of `captions.generate`: source-timed cues that
/// `captions.apply { job }` inserts as one caption item.
#[derive(Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionsResult {
    pub asset: media::AssetId,
    pub range: SourceRange,
    pub source_language: captions::LanguageTag,
    pub cues: Vec<captions::Cue>,
    pub segments: usize,
    /// ASR windows served from the checkpoint cache.
    pub checkpoints: usize,
    pub cost: CostView,
    pub provider: NfcText,
    pub model: NfcText,
}

/// The finished translation of `captions.translate`: exactly the fields the
/// `retranslate` edit takes.
#[derive(Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TranslationsResult {
    pub item: ItemId,
    pub target_language: captions::LanguageTag,
    pub texts: std::collections::BTreeMap<NfcText, NfcText>,
    pub translation_ref: NfcText,
    pub cost: CostView,
    pub provider: NfcText,
    pub model: NfcText,
}

impl std::fmt::Debug for CaptionsResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptionsResult")
            .field("asset", &self.asset)
            .field("cues", &self.cues.len())
            .field("segments", &self.segments)
            .field("checkpoints", &self.checkpoints)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for TranslationsResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TranslationsResult")
            .field("item", &self.item)
            .field("texts", &self.texts.len())
            .finish_non_exhaustive()
    }
}
