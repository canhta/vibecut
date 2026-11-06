//! Defines the feature result views a control response carries.
use super::Revision;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use time::Tick;

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

/// Result of [`FontStore::load`].
#[derive(Clone, Debug, PartialEq, Serialize, schemars::JsonSchema)]
pub struct FontLoad {
    pub family: String,
    pub state: FontLoadState,
    pub origin: Option<FontOrigin>,
    /// The family layout will use: the request when loaded, otherwise the default family.
    pub effective_family: String,
    pub styles: Vec<String>,
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
    AnalyzeShots,
    DetectScenes,
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
