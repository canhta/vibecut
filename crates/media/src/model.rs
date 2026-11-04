use crate::{MediaError, Result};
use ffmpeg_next as ff;
use serde::{Deserialize, Serialize};
use std::fmt;
use time::{FrameRate, Rounding, Tick};

/// Exact positive seconds per container timestamp unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "(i32, i32)", into = "(i32, i32)")]
pub struct TimeBase {
    numerator: i32,
    denominator: i32,
}
impl TimeBase {
    pub fn new(numerator: i32, denominator: i32) -> Result<Self> {
        if numerator <= 0 || denominator <= 0 {
            return Err(MediaError::Invalid("nonpositive time base"));
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }
    pub fn numerator(self) -> i32 {
        self.numerator
    }
    pub fn denominator(self) -> i32 {
        self.denominator
    }
    pub fn ticks(self, units: i64) -> Result<Tick> {
        Ok(Tick::from_rational(
            units,
            i64::from(self.numerator),
            i64::from(self.denominator),
            Rounding::Nearest,
        )?)
    }
    pub fn units(self, ticks: Tick, rounding: Rounding) -> Result<i64> {
        Ok(ticks.to_rational(
            i64::from(self.numerator),
            i64::from(self.denominator),
            rounding,
        )?)
    }
    pub(crate) fn rational(self) -> ff::Rational {
        ff::Rational(self.numerator, self.denominator)
    }
    pub(crate) fn from_ff(value: ff::Rational) -> Result<Self> {
        Self::new(value.numerator(), value.denominator())
    }
}

/// Owned codec parameters including extradata and codec priming; never a borrowed
/// stream pointer. Used unchanged for stream copy or to construct a codec.
pub struct CodecParameters(pub(crate) ff::codec::Parameters);
impl Clone for CodecParameters {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl fmt::Debug for CodecParameters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CodecParameters")
            .field("codec", &self.0.id().name())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamKind {
    Video,
    Audio,
    Other,
}
/// ITU-T H.273 color code points, preserved without guessing at HDR conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub color: ColorInfo,
    pub bit_depth: Option<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

/// Reference-counted encoded access unit. Cloning a packet retains its FFmpeg
/// buffer; payload bytes are not copied. Timestamps already include demux edits.
pub struct Packet {
    pub(crate) inner: ff::Packet,
    pub(crate) time_base: TimeBase,
}
impl Packet {
    pub fn try_clone(&self) -> Result<Self> {
        Ok(Self {
            inner: crate::ffi::reference_packet(&self.inner)?,
            time_base: self.time_base,
        })
    }
    pub fn stream(&self) -> usize {
        self.inner.stream()
    }
    pub fn time_base(&self) -> TimeBase {
        self.time_base
    }
    pub fn pts(&self) -> Result<Option<Tick>> {
        self.inner
            .pts()
            .map(|v| self.time_base.ticks(v))
            .transpose()
    }
    pub fn dts(&self) -> Result<Option<Tick>> {
        self.inner
            .dts()
            .map(|v| self.time_base.ticks(v))
            .transpose()
    }
    pub fn duration(&self) -> Result<Tick> {
        self.time_base.ticks(self.inner.duration())
    }
    pub fn keyframe(&self) -> bool {
        self.inner.is_key()
    }
    pub fn data(&self) -> &[u8] {
        self.inner.data().unwrap_or(&[])
    }
}
impl fmt::Debug for Packet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Packet")
            .field("stream", &self.stream())
            .field("size", &self.inner.size())
            .finish_non_exhaustive()
    }
}

impl TryFrom<(i32, i32)> for TimeBase {
    type Error = MediaError;
    fn try_from(v: (i32, i32)) -> Result<Self> {
        Self::new(v.0, v.1)
    }
}
impl From<TimeBase> for (i32, i32) {
    fn from(v: TimeBase) -> Self {
        (v.numerator, v.denominator)
    }
}
