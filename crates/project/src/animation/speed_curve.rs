//! Speed curves: a piecewise-linear source/output time map in exact ticks, baked from a shape.
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

/// Most keys a curve holds (forty constant-speed segments).
pub const MAX_SPEED_KEYS: usize = 41;

/// Position of a shape's last point: thousandths of the curve's output span.
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

/// Most control points of an editable curve shape.
pub const MAX_SPEED_POINTS: usize = 12;

/// Constant-speed stretches an editable shape is baked into.
const BAKED_STRETCHES: u16 = 40;

/// Baked stretches stay a hair inside the 0.1-100x range so tick rounding
/// never pushes one out of it.
const BAKED_MIN: f64 = 0.1001;
const BAKED_MAX: f64 = 99.99;

/// Fixed-point scale of the stretch speeds used to split the source span.
const WEIGHT_UNIT: f64 = 1_000_000.0;
const WEIGHT_SCALE: i128 = 1_000_000;

/// One control point of an editable curve: the playback speed at `at`
/// thousandths of the curve's output (timeline) span.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpeedPoint {
    pub at: u16,
    pub speed: Speed,
}

const POINT_ORIGIN: SpeedPoint = SpeedPoint {
    at: 0,
    speed: Speed::UNITY,
};

impl SpeedPoint {
    /// Speed at `fraction` (0..=1) of the output span: between two points the
    /// speed follows a smoothstep, so the curve is smooth, flat at every point
    /// and always between its neighbours (the editor plots it on a log axis).
    pub fn interpolate(points: &[Self], fraction: f64) -> f64 {
        let at = fraction * f64::from(SPEED_SPAN_TOTAL);
        for pair in points.windows(2) {
            if let [a, b] = pair
                && at <= f64::from(b.at)
            {
                let (from, to) = (a.speed.as_f64(), b.speed.as_f64());
                let u = ((at - f64::from(a.at)) / f64::from(b.at - a.at)).clamp(0.0, 1.0);
                let weight = u * u * (3.0 - 2.0 * u);
                return from + (to - from) * weight;
            }
        }
        points.last().map_or(1.0, |point| point.speed.as_f64())
    }

    /// Validate a shape: 2..=12 points, the first at 0, the last at 1000,
    /// strictly increasing positions.
    ///
    /// # Errors
    ///
    /// Returns an error for a bad point count, ends or order.
    pub fn validate(points: &[Self]) -> Result<()> {
        if points.len() < 2 || points.len() > MAX_SPEED_POINTS {
            return Err(Error::Invalid("speed curve point count"));
        }
        if points.first().map(|p| p.at) != Some(0)
            || points.last().map(|p| p.at) != Some(SPEED_SPAN_TOTAL)
        {
            return Err(Error::Invalid("speed curve point ends"));
        }
        if points.windows(2).any(|pair| match pair {
            [a, b] => a.at >= b.at,
            _ => true,
        }) {
            return Err(Error::Invalid("speed curve point order"));
        }
        Ok(())
    }
}

/// Named curve shapes, read off the reference editor's preset icons (its
/// Montage points are measured from the editor plot). `None` is no curve and
/// has no entry here; `Custom` starts a flat 1x shape to edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SpeedPreset {
    /// A flat 1x shape, for hand editing.
    Custom,
    /// 0.9x, a fast crest of 2.69x, a slow trough of 0.3x, back to 1x.
    Montage,
    /// Two fast crests around a slow trough.
    Hero,
    /// Fast, a short slow dip, fast again.
    Bullet,
    /// Slow with one fast spike.
    JumpCut,
    /// Starts fast and settles to 1x.
    FlashIn,
    /// Starts at 1x and ends fast.
    FlashOut,
}

impl SpeedPreset {
    pub const ALL: [Self; 7] = [
        Self::Custom,
        Self::Montage,
        Self::Hero,
        Self::Bullet,
        Self::JumpCut,
        Self::FlashIn,
        Self::FlashOut,
    ];

    /// The preset's `(position in thousandths, speed in thousandths)` points.
    fn table(self) -> &'static [(u16, u32)] {
        match self {
            Self::Custom => &[(0, 1000), (1000, 1000)],
            Self::Montage => &[
                (0, 900),
                (100, 900),
                (500, 2690),
                (650, 300),
                (800, 1000),
                (1000, 1000),
            ],
            Self::Hero => &[
                (0, 1000),
                (250, 1800),
                (450, 500),
                (550, 500),
                (750, 1800),
                (1000, 1000),
            ],
            Self::Bullet => &[
                (0, 1800),
                (300, 1800),
                (450, 500),
                (570, 500),
                (740, 1800),
                (1000, 1800),
            ],
            Self::JumpCut => &[(0, 500), (380, 500), (490, 2500), (600, 500), (1000, 500)],
            Self::FlashIn => &[(0, 2000), (320, 2000), (650, 1000), (1000, 1000)],
            Self::FlashOut => &[(0, 1000), (350, 1000), (670, 2000), (1000, 2000)],
        }
    }

    /// The control points this preset creates.
    pub fn points(self) -> Vec<SpeedPoint> {
        self.table()
            .iter()
            .filter_map(|&(at, thousandths)| {
                Speed::from_thousandths(thousandths)
                    .ok()
                    .map(|speed| SpeedPoint { at, speed })
            })
            .collect()
    }
}

/// Validated curve of 2..=41 keys, stored inline so `Retime` stays `Copy`, with
/// the control points (the editable shape) the keys were baked from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "SpeedCurveWire", into = "SpeedCurveWire")]
#[schemars(with = "SpeedCurveWire")]
pub struct SpeedCurve {
    keys: [SpeedKey; MAX_SPEED_KEYS],
    len: u8,
    points: [SpeedPoint; MAX_SPEED_POINTS],
    point_len: u8,
}

/// The stored form of a curve: its keys (the time map) and its control points
/// (the editable shape the keys were baked from).
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SpeedCurveWire {
    keys: Vec<SpeedKey>,
    points: Vec<SpeedPoint>,
}

impl TryFrom<SpeedCurveWire> for SpeedCurve {
    type Error = Error;
    fn try_from(wire: SpeedCurveWire) -> Result<Self> {
        let mut curve = Self::new(&wire.keys)?;
        SpeedPoint::validate(&wire.points)?;
        for (slot, point) in curve.points.iter_mut().zip(&wire.points) {
            *slot = *point;
        }
        curve.point_len = wire.points.len() as u8;
        Ok(curve)
    }
}

impl From<SpeedCurve> for SpeedCurveWire {
    fn from(curve: SpeedCurve) -> Self {
        Self {
            keys: curve.keys().to_vec(),
            points: curve.points().to_vec(),
        }
    }
}

fn overflow() -> Error {
    Error::Invalid("speed curve time overflow")
}

impl SpeedCurve {
    /// Validate keys: the first is `(0, 0)`, both coordinates strictly
    /// increase, and every stretch plays between 0.1x and 100x.
    ///
    /// # Errors
    ///
    /// Returns an error for a bad key count, a non-zero origin, a stretch that
    /// does not advance in both clocks, or one outside the speed range.
    fn new(keys: &[SpeedKey]) -> Result<Self> {
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
            points: [POINT_ORIGIN; MAX_SPEED_POINTS],
            point_len: 0,
        })
    }

    /// Build a smooth curve over `source` ticks from control points. The
    /// shape is a speed over output time: the output span divides into forty
    /// equal stretches, each playing at the shape's speed at its midpoint, and
    /// the output length follows from the source it must cover.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid shape or a source span too short for
    /// forty stretches.
    pub fn from_points(source: Tick, points: &[SpeedPoint]) -> Result<Self> {
        SpeedPoint::validate(points)?;
        let stretches = i128::from(BAKED_STRETCHES);
        let weights: Vec<i128> = (0..BAKED_STRETCHES)
            .map(|n| {
                let middle = (f64::from(n) + 0.5) / f64::from(BAKED_STRETCHES);
                let speed = SpeedPoint::interpolate(points, middle).clamp(BAKED_MIN, BAKED_MAX);
                (speed * WEIGHT_UNIT).round() as i128
            })
            .collect();
        let total: i128 = weights.iter().sum();
        let source_ticks = i128::from(source.0);
        let divide = |numerator: i128, denominator: i128| -> Result<Tick> {
            let rounded = (numerator * 2 + denominator) / (denominator * 2);
            i64::try_from(rounded).map(Tick).map_err(|_| overflow())
        };
        let output_total = divide(source_ticks * stretches * WEIGHT_SCALE, total)?;
        let mut keys = vec![ORIGIN];
        let mut cumulative = 0i128;
        for (n, weight) in (1i128..).zip(&weights) {
            cumulative += weight;
            keys.push(SpeedKey {
                output: divide(i128::from(output_total.0) * n, stretches)?,
                source: divide(source_ticks * cumulative, total)?,
            });
        }
        let mut curve = Self::new(&keys)?;
        for (slot, point) in curve.points.iter_mut().zip(points) {
            *slot = *point;
        }
        curve.point_len = points.len() as u8;
        Ok(curve)
    }

    /// The same shape over another source span, re-baked.
    ///
    /// # Errors
    ///
    /// Returns an error when the curve cannot cover `source`.
    pub fn over(&self, source: Tick) -> Result<Self> {
        Self::from_points(source, self.points())
    }

    /// The shape of the output slice `[offset, offset + duration)`: what a split
    /// piece plays. Its ends are the shape's speeds at the cut, the points inside
    /// the slice are rescaled to its span. The whole shape when the slice is whole.
    pub fn slice_points(&self, offset: Tick, duration: Tick) -> Vec<SpeedPoint> {
        let whole = self.points().to_vec();
        let total = self.total_output().0;
        if total <= 0 || duration.0 <= 0 || (offset.0 <= 0 && duration.0 >= total) {
            return whole;
        }
        let from = (offset.0.max(0) as f64 / total as f64).min(1.0);
        let to = ((offset.0.max(0) + duration.0) as f64 / total as f64).min(1.0);
        let span = to - from;
        if span <= 0.0 {
            return whole;
        }
        let speed_at = |fraction: f64| {
            Speed::from_f64(SpeedPoint::interpolate(&whole, fraction)).unwrap_or(Speed::UNITY)
        };
        let mut slice = vec![SpeedPoint {
            at: 0,
            speed: speed_at(from),
        }];
        for point in &whole {
            let fraction = f64::from(point.at) / f64::from(SPEED_SPAN_TOTAL);
            let at = ((fraction - from) / span * f64::from(SPEED_SPAN_TOTAL)).round();
            let last = slice.last().map_or(0, |p| p.at);
            if at > f64::from(last) && at < f64::from(SPEED_SPAN_TOTAL) {
                slice.push(SpeedPoint {
                    at: at as u16,
                    speed: point.speed,
                });
            }
        }
        slice.push(SpeedPoint {
            at: SPEED_SPAN_TOTAL,
            speed: speed_at(to),
        });
        if SpeedPoint::validate(&slice).is_ok() {
            slice
        } else {
            whole
        }
    }

    /// The editable control points.
    pub fn points(&self) -> &[SpeedPoint] {
        self.points
            .get(..usize::from(self.point_len))
            .unwrap_or(&[])
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
