//! Persistable source identity and probe metadata, without native codec handles.
use crate::{AudioInfo, MediaError, Probe, Result, StreamKind, TimeBase, VideoInfo};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fmt,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use time::{FrameRate, Tick};

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(transparent)]
pub struct AssetId(pub u64);

/// BLAKE3 of all file bytes; never a path or timestamp fingerprint.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct ContentHash(pub [u8; 32]);
impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[content hash]")
    }
}
impl ContentHash {
    /// Hash already bounded in-memory content with the same full-byte identity as read.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }
    pub fn read(path: &Path) -> Result<Self> {
        Self::read_with_cancel(path, || false)
    }
    /// Stream the content identity in 64 KiB chunks, checking cancellation before
    /// every read and before returning the complete digest.
    pub fn read_with_cancel(path: &Path, cancelled: impl Fn() -> bool) -> Result<Self> {
        if cancelled() {
            return Err(MediaError::Cancelled);
        }
        let mut hasher = blake3::Hasher::new();
        let mut file = File::open(path)?;
        let mut bytes = [0u8; 65536];
        loop {
            if cancelled() {
                return Err(MediaError::Cancelled);
            }
            let count = file.read(&mut bytes)?;
            if count == 0 {
                break;
            }
            hasher.update(
                bytes
                    .get(..count)
                    .ok_or(MediaError::Invalid("hash read size"))?,
            );
        }
        if cancelled() {
            return Err(MediaError::Cancelled);
        }
        Ok(Self(*hasher.finalize().as_bytes()))
    }
}

/// Persisted stream metadata; codec parameters remain runtime-only in `Probe`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceStream {
    pub index: usize,
    pub kind: StreamKind,
    pub codec: String,
    pub time_base: TimeBase,
    pub duration: Option<Tick>,
    pub start: Option<Tick>,
    pub frame_rate: Option<FrameRate>,
    pub frame_count: Option<u64>,
    pub variable_frame_rate: Option<bool>,
    pub video: Option<VideoInfo>,
    pub audio: Option<AudioInfo>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceProbe {
    pub format: String,
    pub duration: Option<Tick>,
    pub streams: Vec<SourceStream>,
}
impl From<&Probe> for SourceProbe {
    fn from(probe: &Probe) -> Self {
        Self {
            format: probe.format.clone(),
            duration: probe.duration,
            streams: probe
                .streams
                .iter()
                .map(|s| SourceStream {
                    index: s.index,
                    kind: s.kind,
                    codec: s.codec.clone(),
                    time_base: s.time_base,
                    duration: s.duration,
                    start: s.start,
                    frame_rate: s.frame_rate,
                    frame_count: s.frame_count,
                    variable_frame_rate: s.variable_frame_rate,
                    video: s.video.clone(),
                    audio: s.audio.clone(),
                })
                .collect(),
        }
    }
}

#[derive(Clone, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "AssetData")]
#[schemars(with = "AssetData")]
pub struct SourceAsset(
    AssetData,
    #[serde(skip)]
    #[schemars(skip)]
    pub(crate) Arc<Mutex<crate::vision_prep::MfccAnchors>>,
);
impl Serialize for SourceAsset {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}
impl PartialEq for SourceAsset {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for SourceAsset {}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssetData {
    /// Internal assets remain usable but are omitted from the media library.
    pub ephemeral: bool,
    pub id: AssetId,
    pub path: PathBuf,
    pub content_hash: ContentHash,
    pub probe: SourceProbe,
}
impl fmt::Debug for SourceAsset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourceAsset")
            .field("id", &self.id())
            .finish_non_exhaustive()
    }
}
impl TryFrom<AssetData> for SourceAsset {
    type Error = MediaError;
    fn try_from(data: AssetData) -> Result<Self> {
        // Absolute native paths retain identity even if media is temporarily absent.
        if !data.path.is_absolute()
            || data.path.to_str().is_none_or(|p| p.contains('\0'))
            || data.path.as_os_str().len() > 32768
        {
            return Err(MediaError::Invalid(
                "asset path must be absolute representable Unicode",
            ));
        }
        let p = &data.probe;
        if p.format.len() > 256
            || p.streams.len() > 256
            || p.duration.is_some_and(|d| d < Tick::ZERO)
        {
            return Err(MediaError::Invalid("source probe bounds"));
        }
        let mut indexes = BTreeSet::new();
        for s in &p.streams {
            if !indexes.insert(s.index)
                || s.codec.len() > 256
                || s.duration.is_some_and(|d| d < Tick::ZERO)
            {
                return Err(MediaError::Invalid("source stream metadata"));
            }
            if s.video.as_ref().is_some_and(|v| {
                v.width == 0 || v.height == 0 || u64::from(v.width) * u64::from(v.height) > 1 << 26
            }) || s
                .audio
                .as_ref()
                .is_some_and(|a| a.sample_rate == 0 || a.channels == 0 || a.channels > 32)
                || (s.kind == StreamKind::Video) != s.video.is_some()
                || (s.kind == StreamKind::Audio) != s.audio.is_some()
            {
                return Err(MediaError::Invalid("source stream kind or dimensions"));
            }
        }
        Ok(Self(data, Arc::default()))
    }
}
impl From<SourceAsset> for AssetData {
    fn from(asset: SourceAsset) -> Self {
        asset.0
    }
}
impl SourceAsset {
    pub fn ephemeral(&self) -> bool {
        self.0.ephemeral
    }
    /// Classifies supported image demuxers, including animated GIF/APNG,
    /// from the probed format rather than the filename.
    pub fn is_image(&self) -> bool {
        matches!(
            self.0.probe.format.as_str(),
            "png_pipe" | "jpeg_pipe" | "webp_pipe" | "bmp_pipe" | "tiff_pipe" | "gif" | "apng"
        )
    }
    pub fn id(&self) -> AssetId {
        self.0.id
    }
    pub fn path(&self) -> &Path {
        &self.0.path
    }
    pub fn content_hash(&self) -> ContentHash {
        self.0.content_hash
    }
    pub fn probe(&self) -> &SourceProbe {
        &self.0.probe
    }
    /// Reads and hashes source content; caller controls scheduling for large files.
    pub fn import(id: AssetId, path: PathBuf) -> Result<Self> {
        Self::import_with_cancel(id, path, Arc::new(AtomicBool::new(false)))
    }
    /// Interruptible probe and full-content hash. The token is checked before
    /// canonicalization, is passed into FFmpeg's probe interrupt callback and
    /// checked before every 64 KiB hash read, so leaving a project or cancelling
    /// a batch does not keep reading a large file.
    pub fn import_with_cancel(
        id: AssetId,
        path: PathBuf,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self> {
        if cancelled.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        let path = std::fs::canonicalize(path)?;
        let reader = crate::PacketReader::open_with_cancel(&path, cancelled.clone())?;
        let probe = SourceProbe::from(reader.probe());
        drop(reader);
        if cancelled.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        let content_hash =
            ContentHash::read_with_cancel(&path, || cancelled.load(Ordering::Relaxed))?;
        Self::try_from(AssetData {
            ephemeral: false,
            id,
            path,
            content_hash,
            probe,
        })
    }
    /// A successful relink preserves identity and only accepts identical content.
    pub fn relink(&self, path: PathBuf) -> Result<Self> {
        let path = std::fs::canonicalize(path)?;
        if ContentHash::read(&path)? != self.content_hash() {
            return Err(MediaError::Invalid("relink content differs"));
        }
        let mut data = self.0.clone();
        data.path = path;
        Self::try_from(data)
    }
    pub fn availability(&self) -> Result<AssetAvailability> {
        match std::fs::metadata(self.path()) {
            Ok(meta) if meta.is_file() => Ok(AssetAvailability::Present),
            Ok(_) => Ok(AssetAvailability::Missing),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AssetAvailability::Missing),
            Err(e) => Err(e.into()),
        }
    }
}
/// Presence alone does not establish that content still matches the fingerprint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetAvailability {
    Present,
    Missing,
}
