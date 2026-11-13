//! Speed curves: a piecewise-linear source/output time map in exact ticks.
//!
//! A curve is a short list of keys `(output, source)` measured from the start
//! of the item's retime window. Between two keys the playback speed is
//! constant, so the map is continuous, strictly increasing and invertible, and
//! every lookup is integer arithmetic with an explicit rounding mode. Splitting
//! or trimming keeps the whole curve and slices its output window, exactly as a
//! constant rate does, so each half reproduces the original mapping.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use time::{Rounding, Speed, Tick};

/// Most keys a curve holds (seven constant-speed segments).
pub const MAX_SPEED_KEYS: usize = 8;

/// Total of [`SpeedSegment::span`] over one curve: a thousandth of the source.
pub const SPEED_SPAN_TOTAL: u16 = 1000;

/// One key of a curve: `source` ticks of the window have played after `output` ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpeedKey {
    pub output: Tick,
    pub source: Tick,
}

const ORIGIN: SpeedKey = SpeedKey {
    output: Tick::ZERO,
    source: Tick::ZERO,
};

/// A constant-speed stretch of a curve, as edited: its share of the source
/// span in thousandths and its speed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpeedSegment {
    /// Thousandths of the clip's source span; all spans of one curve sum to 1000.
    pub span: u16,
    pub speed: Speed,
}

/// Named ramps. The ids describe the shape; the reference editor's own preset
/// names are not measured yet (see the editor UI doc).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SpeedPreset {
    /// Starts slow and accelerates through four steps.
    SlowInFastOut,
    /// Starts fast and decelerates through four steps.
    FastInSlowOut,
    /// Normal speed with a slow-motion middle.
    SlowMiddle,
    /// Normal speed with a fast middle.
    FastMiddle,
}

impl SpeedPreset {
    pub const ALL: [Self; 4] = [
        Self::SlowInFastOut,
        Self::FastInSlowOut,
        Self::SlowMiddle,
        Self::FastMiddle,
    ];

    /// The preset's `(span, speed in thousandths)` stretches.
    fn table(self) -> &'static [(u16, u16)] {
        match self {
            Self::SlowInFastOut => &[(250, 500), (250, 800), (250, 1250), (250, 2000)],
            Self::FastInSlowOut => &[(250, 2000), (250, 1250), (250, 800), (250, 500)],
            Self::SlowMiddle => &[(300, 1000), (400, 300), (300, 1000)],
            Self::FastMiddle => &[(300, 1000), (400, 3000), (300, 1000)],
        }
    }

    /// The editable stretches this preset creates.
    pub fn segments(self) -> Vec<SpeedSegment> {
        self.table()
            .iter()
            .filter_map(|&(span, thousandths)| {
                Speed::from_thousandths(thousandths)
                    .ok()
                    .map(|speed| SpeedSegment { span, speed })
            })
            .collect()
    }
}

/// Validated curve of 2..=8 keys, stored inline so `Retime` stays `Copy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "Vec<SpeedKey>", into = "Vec<SpeedKey>")]
#[schemars(with = "Vec<SpeedKey>")]
pub struct SpeedCurve {
    keys: [SpeedKey; MAX_SPEED_KEYS],
    len: u8,
}

impl TryFrom<Vec<SpeedKey>> for SpeedCurve {
    type Error = Error;
    fn try_from(keys: Vec<SpeedKey>) -> Result<Self> {
        Self::new(&keys)
    }
}

impl From<SpeedCurve> for Vec<SpeedKey> {
    fn from(curve: SpeedCurve) -> Self {
        curve.keys().to_vec()
    }
}

fn overflow() -> Error {
    Error::Invalid("speed curve time overflow")
}

impl SpeedCurve {
    /// Validate keys: the first is `(0, 0)`, both coordinates strictly
    /// increase, and every stretch plays between 0.01x and 5x.
    ///
    /// # Errors
    ///
    /// Returns an error for a bad key count, a non-zero origin, a stretch that
    /// does not advance in both clocks, or one outside the speed range.
    pub fn new(keys: &[SpeedKey]) -> Result<Self> {
        if keys.len() < 2 || keys.len() > MAX_SPEED_KEYS {
            return Err(Error::Invalid("speed curve key count"));
        }
        if keys.first() != Some(&ORIGIN) {
            return Err(Error::Invalid("speed curve origin"));
        }
        for pair in keys.windows(2) {
            if let [a, b] = pair {
                let out = i128::from(b.output.0) - i128::from(a.output.0);
                let src = i128::from(b.source.0) - i128::from(a.source.0);
                if out <= 0 || src <= 0 {
                    return Err(Error::Invalid("speed curve must advance"));
                }
                if src * 1000 < out * i128::from(Speed::MIN_THOUSANDTHS)
                    || src * 1000 > out * i128::from(Speed::MAX_THOUSANDTHS)
                {
                    return Err(Error::Invalid("speed curve speed range"));
                }
            }
        }
        let mut stored = [ORIGIN; MAX_SPEED_KEYS];
        for (slot, key) in stored.iter_mut().zip(keys) {
            *slot = *key;
        }
        Ok(Self {
            keys: stored,
            len: keys.len() as u8,
        })
    }

    /// Build a curve over `source` ticks from edited stretches. Each stretch
    /// plays for the ceiling of its source ticks over its speed, so no stretch
    /// exceeds the speed it was given.
    ///
    /// # Errors
    ///
    /// Returns an error when the spans do not sum to 1000, a stretch has no
    /// source tick, there are more than seven stretches, or a key is invalid.
    pub fn from_segments(source: Tick, segments: &[SpeedSegment]) -> Result<Self> {
        if segments.is_empty() || segments.len() >= MAX_SPEED_KEYS {
            return Err(Error::Invalid("speed curve segment count"));
        }
        let total: u32 = segments.iter().map(|s| u32::from(s.span)).sum();
        if total != u32::from(SPEED_SPAN_TOTAL) {
            return Err(Error::Invalid("speed curve spans must sum to 1000"));
        }
        let mut keys = vec![ORIGIN];
        let (mut cumulative, mut output, mut previous) = (0i64, 0i64, 0i64);
        for segment in segments {
            cumulative += i64::from(segment.span);
            let at = source.checked_scale(cumulative, 1000, Rounding::Nearest)?;
            let advance = at.0.checked_sub(previous).ok_or_else(overflow)?;
            if advance <= 0 {
                return Err(Error::Invalid("speed curve must advance"));
            }
            let played = Tick(advance).checked_scale(
                1000,
                i64::from(segment.speed.thousandths()),
                Rounding::Ceil,
            )?;
            output = output.checked_add(played.0).ok_or_else(overflow)?;
            previous = at.0;
            keys.push(SpeedKey {
                output: Tick(output),
                source: at,
            });
        }
        Self::new(&keys)
    }

    /// The curve as editable stretches (spans rounded to the nearest thousandth
    /// of the source, speeds to the nearest 0.001x).
    pub fn segments(&self) -> Vec<SpeedSegment> {
        let total = self.total_source().0.max(1);
        let mut out = Vec::new();
        let mut before = 0i64;
        for pair in self.keys().windows(2) {
            if let [a, b] = pair {
                let cumulative = Tick(b.source.0)
                    .checked_scale(1000, total, Rounding::Nearest)
                    .map_or(before, |t| t.0);
                let src = (b.source.0 - a.source.0) as f64;
                let played = (b.output.0 - a.output.0) as f64;
                let speed = Speed::from_f64(src / played).unwrap_or(Speed::UNITY);
                out.push(SpeedSegment {
                    span: u16::try_from(cumulative - before).unwrap_or(0),
                    speed,
                });
                before = cumulative;
            }
        }
        out
    }

    pub fn keys(&self) -> &[SpeedKey] {
        self.keys.get(..usize::from(self.len)).unwrap_or(&[])
    }

    /// Output ticks of the whole window.
    pub fn total_output(&self) -> Tick {
        self.keys().last().map_or(Tick::ZERO, |k| k.output)
    }

    /// Source ticks of the whole window.
    pub fn total_source(&self) -> Tick {
        self.keys().last().map_or(Tick::ZERO, |k| k.source)
    }

    /// The stretch containing `at`; outside the curve the nearest end stretch
    /// extends linearly.
    fn stretch(&self, at: Tick, by_output: bool) -> Result<(SpeedKey, SpeedKey)> {
        let mut found = None;
        for pair in self.keys().windows(2) {
            if let [a, b] = pair {
                found = Some((*a, *b));
                let end = if by_output { b.output } else { b.source };
                if at < end {
                    break;
                }
            }
        }
        found.ok_or(Error::Invalid("speed curve stretch"))
    }

    /// Source ticks played after `local` output ticks of the window.
    ///
    /// # Errors
    ///
    /// Returns an error on arithmetic overflow.
    pub fn source_offset(&self, local: Tick, rounding: Rounding) -> Result<Tick> {
        let (a, b) = self.stretch(local, true)?;
        let out = b.output.0 - a.output.0;
        let src = b.source.0 - a.source.0;
        let rel = Tick(local.0.checked_sub(a.output.0).ok_or_else(overflow)?);
        let part = rel.checked_scale(src, out, rounding)?;
        a.source.checked_add(part).ok_or_else(overflow)
    }

    /// Output ticks at which `source` ticks of the window have played.
    ///
    /// # Errors
    ///
    /// Returns an error on arithmetic overflow.
    pub fn output_offset(&self, source: Tick, rounding: Rounding) -> Result<Tick> {
        let (a, b) = self.stretch(source, false)?;
        let out = b.output.0 - a.output.0;
        let src = b.source.0 - a.source.0;
        let rel = Tick(source.0.checked_sub(a.source.0).ok_or_else(overflow)?);
        let part = rel.checked_scale(out, src, rounding)?;
        a.output.checked_add(part).ok_or_else(overflow)
    }
}
