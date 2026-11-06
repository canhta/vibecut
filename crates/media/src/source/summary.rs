//! Import-facing media metadata and codec capability checks.
//!
//! [MediaMetadata] is derived from the persistable [SourceProbe] and therefore
//! survives a project round trip. [ProbeCapabilities] opens decoders without
//! decoding and stays transient: it describes this machine's FFmpeg build, not
//! the media file.
use crate::{
    AudioDecoder, DecodePreference, MediaError, Probe, Result, SourceAsset, SourceProbe,
    StreamInfo, StreamKind, VideoDecoder,
};
use serde::{Deserialize, Serialize};
use time::{FrameRate, Tick};

/// The library category shown for an imported source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Video,
    Image,
    Audio,
}

/// Exact rational frame rate plus the integer a creator sees in compact rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DisplayFrameRate {
    /// The exact source rate, never rounded (for example `24000/1001`).
    pub exact: FrameRate,
    /// Nearest integer frames per second chosen for compact display (24).
    pub rounded: u32,
}

impl DisplayFrameRate {
    /// Round to the nearest integer for display; the exact rate is retained.
    pub fn new(exact: FrameRate) -> Result<Self> {
        let value = exact.as_f64();
        if !value.is_finite() || value <= 0.0 || value >= f64::from(u32::MAX) {
            return Err(MediaError::Invalid("frame rate for display"));
        }
        Ok(Self {
            exact,
            rounded: value.round() as u32,
        })
    }
}

/// Persistable, host-independent summary of what a source contains.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MediaMetadata {
    pub kind: MediaKind,
    pub duration: Option<Tick>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub display_frame_rate: Option<DisplayFrameRate>,
    pub has_audio: bool,
}

impl MediaMetadata {
    pub fn from_probe(probe: &SourceProbe) -> Result<Self> {
        let video = probe
            .streams
            .iter()
            .find(|stream| stream.kind == StreamKind::Video);
        let audio = probe
            .streams
            .iter()
            .find(|stream| stream.kind == StreamKind::Audio);
        let kind = classify(&probe.format, video.is_some(), audio.is_some());
        let display_frame_rate = video
            .and_then(|stream| stream.frame_rate)
            .map(DisplayFrameRate::new)
            .transpose()?;
        Ok(Self {
            kind,
            duration: probe.duration,
            width: video.and_then(|stream| stream.video.as_ref().map(|video| video.width)),
            height: video.and_then(|stream| stream.video.as_ref().map(|video| video.height)),
            display_frame_rate,
            has_audio: audio.is_some(),
        })
    }

    /// The first thumbnail is generated from the first decodable frame, so only
    /// still images and videos carry one.
    pub fn needs_thumbnail(&self) -> bool {
        matches!(self.kind, MediaKind::Video | MediaKind::Image)
    }
}

/// FFmpeg's image demuxers are named per format; a still image decodes as one
/// video frame and records dimensions like any picture.
fn classify(format: &str, has_video: bool, has_audio: bool) -> MediaKind {
    if has_video && !has_audio && is_image_format(format) {
        MediaKind::Image
    } else if has_video {
        MediaKind::Video
    } else if has_audio {
        MediaKind::Audio
    } else {
        MediaKind::Video
    }
}

pub(crate) fn is_image_format(format: &str) -> bool {
    matches!(
        format,
        "png_pipe" | "jpeg_pipe" | "webp_pipe" | "gif" | "apng" | "bmp_pipe" | "tiff_pipe"
    )
}

/// Why a codec cannot be used on this machine. Capability based, never a
/// filename or browser hint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedReason {
    /// This FFmpeg build has no decoder for the stream.
    DecoderUnavailable,
    /// The decoder exists but refused to open for this stream.
    OpenFailed,
}

impl UnsupportedReason {
    /// Stable diagnostic; the app maps it through its own catalog.
    pub const fn label(self) -> &'static str {
        match self {
            Self::DecoderUnavailable => "decoder unavailable in this FFmpeg build",
            Self::OpenFailed => "codec could not be opened",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case", tag = "state", content = "reason")]
pub enum Decodability {
    Decodable,
    Unsupported(UnsupportedReason),
}

impl Decodability {
    pub fn is_decodable(self) -> bool {
        matches!(self, Self::Decodable)
    }
}

/// One audio or video stream's transient decoder availability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StreamCapability {
    pub index: usize,
    pub kind: StreamKind,
    pub codec: String,
    pub decodability: Decodability,
}

/// Result of opening (not decoding) every audio/video decoder in the probe.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProbeCapabilities {
    pub streams: Vec<StreamCapability>,
}

impl ProbeCapabilities {
    /// Open each audio/video decoder once. No packet is decoded, so this stays
    /// bounded and independent of file length.
    pub fn from_probe(probe: &Probe) -> Self {
        Self {
            streams: probe
                .streams
                .iter()
                .filter(|stream| matches!(stream.kind, StreamKind::Video | StreamKind::Audio))
                .map(stream_capability)
                .collect(),
        }
    }

    /// Explicitly decodable capabilities for tests and callers that already
    /// validated the source.
    pub fn decodable(streams: impl IntoIterator<Item = (usize, StreamKind, String)>) -> Self {
        Self {
            streams: streams
                .into_iter()
                .map(|(index, kind, codec)| StreamCapability {
                    index,
                    kind,
                    codec,
                    decodability: Decodability::Decodable,
                })
                .collect(),
        }
    }

    /// The first capability failure, if any. A video with a missing audio
    /// decoder is still reported because the asset claims that audio stream.
    pub fn unsupported(&self) -> Option<UnsupportedReason> {
        self.streams
            .iter()
            .find_map(|stream| match stream.decodability {
                Decodability::Decodable => None,
                Decodability::Unsupported(reason) => Some(reason),
            })
    }

    pub fn is_decodable(&self) -> bool {
        self.unsupported().is_none()
    }

    pub fn video(&self) -> Option<&StreamCapability> {
        self.streams
            .iter()
            .find(|stream| stream.kind == StreamKind::Video)
    }

    pub fn audio(&self) -> Option<&StreamCapability> {
        self.streams
            .iter()
            .find(|stream| stream.kind == StreamKind::Audio)
    }
}

fn stream_capability(stream: &StreamInfo) -> StreamCapability {
    let decodability = match stream.kind {
        StreamKind::Video => VideoDecoder::new(stream, DecodePreference::Software)
            .map(|_| Decodability::Decodable)
            .unwrap_or_else(|error| Decodability::Unsupported(reason(&error))),
        StreamKind::Audio => AudioDecoder::new(stream)
            .map(|_| Decodability::Decodable)
            .unwrap_or_else(|error| Decodability::Unsupported(reason(&error))),
        StreamKind::Other => Decodability::Decodable,
    };
    StreamCapability {
        index: stream.index,
        kind: stream.kind,
        codec: stream.codec.clone(),
        decodability,
    }
}

fn reason(error: &MediaError) -> UnsupportedReason {
    match error {
        MediaError::Unsupported(_) => UnsupportedReason::DecoderUnavailable,
        _ => UnsupportedReason::OpenFailed,
    }
}

/// One completed probe: identity/metadata to persist plus this build's
/// transient decoder capabilities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedSource {
    pub asset: SourceAsset,
    pub capabilities: ProbeCapabilities,
}

impl ImportedSource {
    pub fn metadata(&self) -> Result<MediaMetadata> {
        MediaMetadata::from_probe(self.asset.probe())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn image_classification_retains_import_policy_demuxer_boundaries() {
        for (format, image) in [
            ("png_pipe", true),
            ("jpeg_pipe", true),
            ("gif", true),
            ("apng", true),
            ("mjpeg", false),
            ("image2", false),
        ] {
            let asset = SourceAsset::try_from(crate::AssetData {
                ephemeral: false,
                id: crate::AssetId(1),
                path: std::env::temp_dir().join("image-classification"),
                content_hash: crate::ContentHash([0; 32]),
                probe: SourceProbe {
                    format: format.into(),
                    duration: None,
                    streams: Vec::new(),
                },
            })
            .unwrap();
            assert_eq!(asset.is_image(), image, "{format}");
        }
    }
}
