//! Describes a probed container and the streams it carries.

use serde::{Deserialize, Serialize};
use time::{FrameRate, Tick};

use super::model::{CodecParameters, TimeBase};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum StreamKind {
    Video,
    Audio,
    Other,
}
/// ITU-T H.273 color code points, preserved without guessing at HDR conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ColorInfo {
    pub primaries: u32,
    pub transfer: u32,
    pub matrix: u32,
    pub range: u32,
    pub chroma_location: u32,
}
impl Default for ColorInfo {
    fn default() -> Self {
        Self {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            range: 1,
            chroma_location: 1,
        }
    }
}
impl ColorInfo {
    /// Unknown SDR tags resolve to the project's BT.709 limited-range default.
    pub fn resolved_sdr(mut self) -> Self {
        if self.primaries == 2 {
            self.primaries = 1;
        }
        if self.transfer == 2 {
            self.transfer = 1;
        }
        if self.matrix == 2 {
            self.matrix = 1;
        }
        if self.range == 0 {
            self.range = 1;
        }
        if self.chroma_location == 0 {
            self.chroma_location = 1;
        }
        self
    }
    pub fn is_hdr(self) -> bool {
        matches!(self.transfer, 16 | 18)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub color: ColorInfo,
    pub bit_depth: Option<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AudioInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub initial_padding: u32,
    pub trailing_padding: u32,
}
#[derive(Clone, Debug)]
pub struct StreamInfo {
    pub index: usize,
    pub kind: StreamKind,
    pub codec: String,
    pub time_base: TimeBase,
    pub duration: Option<Tick>,
    pub start: Option<Tick>,
    pub frame_rate: Option<FrameRate>,
    pub frame_count: Option<u64>,
    /// None means not established. Comparing nominal/average rates is not proof.
    pub variable_frame_rate: Option<bool>,
    pub video: Option<VideoInfo>,
    pub audio: Option<AudioInfo>,
    pub parameters: CodecParameters,
}
#[derive(Clone, Debug)]
pub struct Probe {
    pub format: String,
    pub duration: Option<Tick>,
    pub streams: Vec<StreamInfo>,
    pub bytes_read: u64,
}
