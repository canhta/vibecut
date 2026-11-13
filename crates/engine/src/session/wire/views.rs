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
    /// A validated font explicitly copied into the user's font store.
    Custom,
}

/// Whether a family can render now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FontState {
    Available,
    /// A catalog family whose files are not in the verified local cache.
    NotDownloaded,
}

/// The active brand kit: its contents only, never local paths. Entries whose file is
/// missing or corrupt stay listed with a state, so the kit is never silently reduced.
#[derive(Clone, Debug, PartialEq, Serialize, schemars::JsonSchema)]
pub struct BrandKitView {
    pub id: u64,
    pub name: String,
    pub colors: Vec<BrandColorView>,
    /// Font family names, resolved by the font store when a project uses them.
    pub fonts: Vec<String>,
    pub media: Vec<BrandMediaView>,
    pub text_presets: Vec<project::SavedTextPreset>,
    pub caption_presets: Vec<project::SavedCaptionPreset>,
}

/// One named brand colour as RGBA bytes.
#[derive(Clone, Debug, PartialEq, Serialize, schemars::JsonSchema)]
pub struct BrandColorView {
    pub name: String,
    pub rgba: [u8; 4],
}

/// One stored media file of the kit.
#[derive(Clone, Debug, PartialEq, Serialize, schemars::JsonSchema)]
pub struct BrandMediaView {
    pub id: u64,
    pub name: String,
    /// Lowercase SHA-256 of the stored bytes, which is also the stored file's name.
    pub sha256: String,
    pub size: u64,
    pub state: BrandEntryState,
}

/// Whether a stored kit file is present and matches its recorded hash and size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrandEntryState {
    Available,
    Missing,
    Corrupt,
}

/// One family in a listing.
#[derive(Clone, Debug, PartialEq, Serialize, schemars::JsonSchema)]
pub struct FontEntry {
    pub family: String,
    pub origin: FontOrigin,
    /// Style names such as `Regular` or `Bold Italic`, sorted by weight.
    pub styles: Vec<String>,
    pub state: FontState,
    /// SPDX licence id for bundled/catalog families, or a user-supplied rights acknowledgement.
    pub license: Option<String>,
    /// Upstream URL, or a local content hash for a custom font; never a source path.
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

/// Where a music search page came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MusicSource {
    Remote,
    Local,
}

/// One music search result; also the record `music.save` upserts. Attribution and
/// the admitted licence identity are precomputed by the library so every surface
/// and export shows the same text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MusicEntry {
    pub id: String,
    pub title: String,
    pub author: String,
    pub licence: String,
    pub licence_url: String,
    pub attribution: String,
    pub source_url: String,
    pub audio_url: String,
    pub duration_ms: u64,
}

/// One bounded page of music search results.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MusicPage {
    pub source: MusicSource,
    pub count: u64,
    pub page: u32,
    pub page_size: u32,
    pub has_next: bool,
    pub results: Vec<MusicEntry>,
}

/// The record a `music.save` upsert produced.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SavedMusicEntry {
    pub music: MusicEntry,
    pub saved_at_unix_ms: u64,
}

/// Filter of `stickers.search`. Stickers are flags only, so `all` and `flags` list the same rows.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum StickerCategory {
    #[default]
    All,
    Flags,
}

/// One bundled sticker: the `provider:value` id a project stores, a display name and the
/// intrinsic pixel size captured at insert.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StickerEntry {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
}

/// One bounded page of sticker search results; `total` counts matches before the limit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StickerPage {
    pub total: u64,
    pub results: Vec<StickerEntry>,
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
    ImportLut,
    Transcribe,
    Translate,
    SuggestClips,
    GenerateChapters,
    ClassifyFillers,
    SelectEmphasis,
    AnalyzeShots,
    AnalyzeReframe,
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

/// One cue's model-chosen emphasis words. `captions.suggest_emphasis` produces these;
/// `captions.set_emphasis` consumes `item`, `cue` and `word_ids`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmphasisSuggestion {
    pub item: ItemId,
    /// The cue the words belong to.
    pub cue: NfcText,
    /// Ascending indices into the cue's verified word records.
    pub word_ids: Vec<u32>,
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

/// Output aspect of a constant auto-reframe crop window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum ReframeTarget {
    #[serde(rename = "9:16")]
    Vertical,
    #[serde(rename = "1:1")]
    Square,
    #[serde(rename = "16:9")]
    Landscape,
}

/// Why a shot's path is not simply the followed face; a shot may carry several.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ReframeReason {
    /// More than one face was stable at once; the largest is followed.
    Ambiguous,
    /// No face was stable; the crop is centred for the whole shot.
    NoSubject,
    /// The followed face was gone for more than 0.5 s and the crop eased to centre.
    Lost,
}

/// One analysed shot of `reframe.generate`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReframeShotReport {
    /// First source tick of the shot.
    pub start: Tick,
    /// Position keys the analysed path carries for this shot, sampled at 8 Hz
    /// before the per-item half-pixel reduction `reframe.apply` performs.
    pub keys: usize,
    pub reasons: Vec<ReframeReason>,
}

/// The retained report of a completed `reframe.generate`; it edits no project.
/// `reframe.apply { job }` reads the same job's full retained analysis instead.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReframeReport {
    pub target: ReframeTarget,
    /// Constant crop width and height as fractions of the source frame.
    pub size: [f64; 2],
    pub shots: Vec<ReframeShotReport>,
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
