//! Models exact container time and the encoded units shared by demux, decode and mux.

use crate::{MediaError, Result};
use ffmpeg_next as ff;
use serde::{Deserialize, Serialize};
use std::fmt;
use time::{Rounding, Tick};

/// Exact positive seconds per container timestamp unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
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
impl CodecParameters {
    /// True for non-square pixels or a nonidentity display matrix. Consumers must
    /// apply that transform or explicitly refuse compositing; stream copy retains it.
    pub fn display_transform_required(&self) -> Result<bool> {
        crate::ffi::display_transform_required(&self.0)
    }
}
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
    /// Retain encoded bytes and shift presentation/decode timestamps by exact
    /// ticks. Used only after the export owner validates safe keyframe boundaries.
    pub fn shifted(&self, delta: Tick) -> Result<Self> {
        let offset = self.time_base.units(delta, Rounding::Nearest)?;
        let shift = |value: Option<i64>| {
            value
                .map(|v| {
                    v.checked_add(offset)
                        .ok_or(MediaError::TooLarge("packet timestamp shift"))
                })
                .transpose()
        };
        let mut packet = self.try_clone()?;
        packet.inner.set_pts(shift(self.inner.pts())?);
        packet.inner.set_dts(shift(self.inner.dts())?);
        Ok(packet)
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
