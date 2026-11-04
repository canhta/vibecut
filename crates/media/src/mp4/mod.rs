//! ISO Base Media File Format (ISO/IEC 14496-12) containers: MP4 and QuickTime MOV.

pub(crate) mod boxes;
pub(crate) mod codec;
pub(crate) mod demux;
pub(crate) mod mux;

use std::fmt;

use time::Tick;

/// A four-character code: a box type, sample-entry format or handler type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct FourCc(pub [u8; 4]);

impl FourCc {
    pub const fn new(bytes: &[u8; 4]) -> Self {
        Self(*bytes)
    }

    /// The code as a big-endian integer.
    pub fn as_u32(self) -> u32 {
        u32::from_be_bytes(self.0)
    }

    /// Printable form; `©` (0xA9) is kept and other non-printable bytes become `?`.
    pub fn to_string_lossy(self) -> String {
        self.0
            .iter()
            .map(|&b| match b {
                0xA9 => '©',
                0x20..=0x7E => char::from(b),
                _ => '?',
            })
            .collect()
    }
}

impl fmt::Display for FourCc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_lossy())
    }
}

impl fmt::Debug for FourCc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FourCc({:?})", self.to_string_lossy())
    }
}

impl From<&[u8; 4]> for FourCc {
    fn from(bytes: &[u8; 4]) -> Self {
        Self(*bytes)
    }
}

impl PartialEq<[u8; 4]> for FourCc {
    fn eq(&self, other: &[u8; 4]) -> bool {
        &self.0 == other
    }
}

impl PartialEq<&[u8; 4]> for FourCc {
    fn eq(&self, other: &&[u8; 4]) -> bool {
        &self.0 == *other
    }
}

/// Broad classification of a stream by its track handler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamKind {
    Video,
    Audio,
    Other,
}

/// Visual parameters of a video stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct VideoInfo {
    /// Coded width in pixels (`tkhd` presentation width).
    pub width: u16,
    /// Coded height in pixels (`tkhd` presentation height).
    pub height: u16,
}

/// Audio parameters of a sound stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct AudioInfo {
    pub sample_rate: u32,
    pub channels: u16,
}

/// One raw edit list entry (`elst`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edit {
    /// Length of the edit in the movie timescale.
    pub segment_duration: u64,
    /// Start within the media in the track timescale; `-1` is an empty edit.
    pub media_time: i64,
    /// Media rate as 16.16 fixed point (`0x0001_0000` is normal speed).
    pub media_rate: i32,
}

/// Summary of one stream and its sample table.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamInfo {
    /// Track ID from `tkhd`.
    pub id: u32,
    pub kind: StreamKind,
    /// Sample-entry format (`avc1`, `mp4a`, `hvc1`, …).
    pub codec: FourCc,
    /// Media timescale (`mdhd`): units per second for this stream's timestamps.
    pub timescale: u32,
    /// Media duration in ticks.
    pub duration: Tick,
    /// Number of samples in the sample table.
    pub sample_count: usize,
    /// Samples marked as sync (keyframes).
    pub keyframe_count: usize,
    /// Video only: sample durations are not all equal.
    pub vfr: bool,
    pub video: Option<VideoInfo>,
    pub audio: Option<AudioInfo>,
    /// `avcC` configuration when the stream is H.264.
    pub avc: Option<AvcConfig>,
    /// `esds` configuration when the stream is AAC.
    pub aac: Option<AacConfig>,
    /// Edit list entries in container units; empty when the track has none.
    pub edits: Vec<Edit>,
}

/// Container-level summary of a probed file.
#[derive(Clone, Debug, PartialEq)]
pub struct Probe {
    /// Major brand from `ftyp` (`isom`, `qt  `, …), or zeros when absent.
    pub brand: FourCc,
    /// Movie timescale (`mvhd`).
    pub timescale: u32,
    /// Movie duration in ticks.
    pub duration: Tick,
    pub streams: Vec<StreamInfo>,
}

pub use crate::mp4::codec::{AacConfig, AvcConfig};

#[cfg(test)]
pub(crate) mod test_support {
    use super::mux::{AudioTrackConfig, Muxer, VideoTrackConfig, WritePacket};
    use super::{AacConfig, AvcConfig};
    use crate::{Brand, MuxOptions};

    /// A small two-track MP4 used by unit tests: 30 video and 30 audio samples,
    /// AAC priming 1024 and a keyframe every ten video samples.
    pub(crate) fn sample_file() -> Vec<u8> {
        let avc = AvcConfig::new(vec![vec![0x67, 0x42, 0, 0x1E]], vec![vec![0x68, 0xCE]], 4);
        let aac = AacConfig::from_asc(vec![0x11, 0x90]);
        let mut muxer = Muxer::new(
            std::io::Cursor::new(Vec::new()),
            MuxOptions {
                brand: Brand::Mp4,
                movie_timescale: 1000,
            },
        )
        .unwrap();
        let video = muxer
            .add_video(VideoTrackConfig {
                width: 640,
                height: 360,
                timescale: 30_000,
                avc,
            })
            .unwrap();
        let audio = muxer
            .add_audio(AudioTrackConfig {
                aac,
                channels: 2,
                sample_rate: 48_000,
                timescale: 48_000,
                priming: 1024,
            })
            .unwrap();
        for index in 0..30u32 {
            muxer
                .write_packet(
                    video,
                    WritePacket {
                        dts: i64::from(index) * 1001,
                        pts: i64::from(index) * 1001,
                        duration: 1001,
                        keyframe: index % 10 == 0,
                        data: &[index as u8; 12],
                    },
                )
                .unwrap();
            muxer
                .write_packet(
                    audio,
                    WritePacket {
                        dts: i64::from(index) * 1024,
                        pts: i64::from(index) * 1024,
                        duration: 1024,
                        keyframe: true,
                        data: &[index as u8; 20],
                    },
                )
                .unwrap();
        }
        muxer.finish().unwrap().into_inner()
    }
}
