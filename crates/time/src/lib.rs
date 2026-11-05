//! Exact integer media time: ticks, rational frame rates and source ranges.
//!
//! Time is an integer count of [`Tick`]s at [`TICKS_PER_SECOND`] = 254 016 000 000
//! per second. That rate divides evenly into every admitted broadcast frame
//! duration and common audio sample duration, so frame and sample conversions are
//! exact. Floating seconds appear only at display and provider boundaries.
//!
//! Fallible operations report [`TimeError`]; hostile input never panics.

mod speed;
mod timecode;

pub use speed::Speed;
pub use timecode::{format_timecode, parse_timecode};

use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

/// Ticks per second.
pub const TICKS_PER_SECOND: i64 = 254_016_000_000;

/// A point or duration in time, in ticks.
///
/// Operator arithmetic saturates at the `i64` limits; use the checked methods when
/// overflow must be reported.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
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
pub struct Tick(pub i64);

impl Tick {
    /// Zero time.
    pub const ZERO: Self = Self(0);

    /// Convert floating seconds to ticks with an explicit rounding mode.
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::NonFiniteSeconds`] for NaN or infinity and
    /// [`TimeError::OutOfRange`] when the rounded tick count does not fit `i64`.
    pub fn from_seconds_f64(seconds: f64, rounding: Rounding) -> Result<Self, TimeError> {
        if !seconds.is_finite() {
            return Err(TimeError::NonFiniteSeconds);
        }
        let scaled = seconds * TICKS_PER_SECOND as f64;
        if !scaled.is_finite() {
            return Err(TimeError::OutOfRange);
        }
        let rounded = match rounding {
            Rounding::Floor => scaled.floor(),
            Rounding::Ceil => scaled.ceil(),
            Rounding::Nearest => scaled.round(),
        };
        if !(I64_MIN_AS_F64..I64_MAX_EXCLUSIVE_AS_F64).contains(&rounded) {
            return Err(TimeError::OutOfRange);
        }
        Ok(Self(rounded as i64))
    }

    /// This time in seconds.
    pub fn seconds(self) -> f64 {
        self.0 as f64 / TICKS_PER_SECOND as f64
    }

    /// Convert a count of `units` at `per_second` (for example audio samples at
    /// 48 000) to ticks with an explicit rounding mode.
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::InvalidRate`] when `per_second` is not positive and
    /// [`TimeError::Overflow`] or [`TimeError::OutOfRange`] when the result cannot
    /// be represented.
    pub fn from_units(units: i64, per_second: i64, rounding: Rounding) -> Result<Self, TimeError> {
        Self::from_rational(units, 1, per_second, rounding)
    }

    /// Convert to a count of units at `per_second` with an explicit rounding mode.
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::InvalidRate`] when `per_second` is not positive and
    /// [`TimeError::Overflow`] or [`TimeError::OutOfRange`] when the result cannot
    /// be represented.
    pub fn to_units(self, per_second: i64, rounding: Rounding) -> Result<i64, TimeError> {
        self.to_rational(1, per_second, rounding)
    }

    /// Convert a rational timestamp `pts * num / den` seconds (a container
    /// timebase) to ticks with an explicit rounding mode.
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::InvalidRate`] when `num` or `den` is not positive and
    /// [`TimeError::Overflow`] or [`TimeError::OutOfRange`] when the result cannot
    /// be represented.
    pub fn from_rational(
        pts: i64,
        num: i64,
        den: i64,
        rounding: Rounding,
    ) -> Result<Self, TimeError> {
        if num <= 0 || den <= 0 {
            return Err(TimeError::InvalidRate);
        }
        let numerator = i128::from(pts)
            .checked_mul(i128::from(num))
            .and_then(|value| value.checked_mul(i128::from(TICKS_PER_SECOND)))
            .ok_or(TimeError::Overflow)?;
        let value = divide_rounded(numerator, i128::from(den), rounding)?;
        i64::try_from(value)
            .map(Self)
            .map_err(|_| TimeError::OutOfRange)
    }

    /// Inverse of [`Tick::from_rational`]: convert this time to a rational
    /// timestamp `num / den` seconds with an explicit rounding mode.
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::InvalidRate`] when `num` or `den` is not positive and
    /// [`TimeError::Overflow`] or [`TimeError::OutOfRange`] when the result cannot
    /// be represented.
    pub fn to_rational(self, num: i64, den: i64, rounding: Rounding) -> Result<i64, TimeError> {
        if num <= 0 || den <= 0 {
            return Err(TimeError::InvalidRate);
        }
        let numerator = i128::from(self.0)
            .checked_mul(i128::from(den))
            .ok_or(TimeError::Overflow)?;
        let denominator = i128::from(TICKS_PER_SECOND)
            .checked_mul(i128::from(num))
            .ok_or(TimeError::Overflow)?;
        let value = divide_rounded(numerator, denominator, rounding)?;
        i64::try_from(value).map_err(|_| TimeError::OutOfRange)
    }

    /// Scale a tick count by a positive rational, using the shared integer rounding policy.
    pub fn checked_scale(self, num: i64, den: i64, rounding: Rounding) -> Result<Self, TimeError> {
        if num <= 0 || den <= 0 {
            return Err(TimeError::InvalidRate);
        }
        let value = divide_rounded(
            i128::from(self.0) * i128::from(num),
            i128::from(den),
            rounding,
        )?;
        i64::try_from(value)
            .map(Self)
            .map_err(|_| TimeError::OutOfRange)
    }

    /// Checked addition.
    pub fn checked_add(self, other: Self) -> Option<Self> {
        self.0.checked_add(other.0).map(Self)
    }

    /// Checked subtraction.
    pub fn checked_sub(self, other: Self) -> Option<Self> {
        self.0.checked_sub(other.0).map(Self)
    }

    /// Checked negation.
    pub fn checked_neg(self) -> Option<Self> {
        self.0.checked_neg().map(Self)
    }

    /// Saturating addition.
    pub fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    /// Saturating subtraction.
    pub fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    /// Saturating negation.
    pub fn saturating_neg(self) -> Self {
        Self(self.0.saturating_neg())
    }

    /// Absolute value, saturating at [`i64::MAX`] for [`i64::MIN`].
    pub fn abs(self) -> Self {
        Self(self.0.saturating_abs())
    }

    /// The smaller of two times.
    pub fn min(self, other: Self) -> Self {
        Self(self.0.min(other.0))
    }

    /// The larger of two times.
    pub fn max(self, other: Self) -> Self {
        Self(self.0.max(other.0))
    }

    /// Clamp to `[low, high]`, returning `high` when the bounds are crossed.
    pub fn clamp(self, low: Self, high: Self) -> Self {
        Self(self.0.max(low.0).min(high.0))
    }
}

impl Add for Tick {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        self.saturating_add(other)
    }
}

impl Sub for Tick {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        self.saturating_sub(other)
    }
}

impl Neg for Tick {
    type Output = Self;

    fn neg(self) -> Self {
        self.saturating_neg()
    }
}

impl AddAssign for Tick {
    fn add_assign(&mut self, other: Self) {
        *self = self.saturating_add(other);
    }
}

impl SubAssign for Tick {
    fn sub_assign(&mut self, other: Self) {
        *self = self.saturating_sub(other);
    }
}

/// Rounding applied when a conversion falls between two ticks or units.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Rounding {
    /// Toward negative infinity.
    Floor,
    /// Toward positive infinity.
    Ceil,
    /// To the nearest value, with ties away from zero.
    Nearest,
}

/// Failure from a time conversion or range constructor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeError {
    /// The frame rate or timecode mode is unsupported.
    Unsupported,
    /// A timecode label is malformed, out of bounds or skipped in DF counting.
    InvalidTimecode,
    /// Floating seconds were NaN or infinite.
    NonFiniteSeconds,
    /// A rate, denominator or timebase was zero or negative.
    InvalidRate,
    /// A range end was not greater than its start.
    EmptyRange,
    /// An intermediate value exceeded its storage width.
    Overflow,
    /// A representable intermediate result did not fit the return type.
    OutOfRange,
    /// A playback speed was not finite or fell outside 0.01x..=5x.
    InvalidSpeed,
}

impl fmt::Display for TimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Unsupported => "unsupported timecode rate or mode",
            Self::InvalidTimecode => "invalid or skipped timecode label",
            Self::NonFiniteSeconds => "seconds must be finite",
            Self::InvalidRate => "rate must be positive",
            Self::EmptyRange => "range end must be greater than start",
            Self::Overflow => "time arithmetic overflowed",
            Self::OutOfRange => "time value is out of range",
            Self::InvalidSpeed => "speed must be between 0.01x and 5x",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for TimeError {}

/// A frame rate as an exact rational `num / den` frames per second.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "FrameRateWire")]
pub struct FrameRate {
    num: i64,
    den: i64,
}

impl Ord for FrameRate {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (i128::from(self.num) * i128::from(other.den))
            .cmp(&(i128::from(other.num) * i128::from(self.den)))
    }
}
impl PartialOrd for FrameRate {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Default for FrameRate {
    fn default() -> Self {
        Self::FPS_23_976
    }
}

impl FrameRate {
    /// 23.976 fps (`24000/1001`).
    pub const FPS_23_976: Self = Self {
        num: 24_000,
        den: 1_001,
    };
    /// 24 fps.
    pub const FPS_24: Self = Self { num: 24, den: 1 };
    /// 25 fps.
    pub const FPS_25: Self = Self { num: 25, den: 1 };
    /// 29.97 fps (`30000/1001`).
    pub const FPS_29_97: Self = Self {
        num: 30_000,
        den: 1_001,
    };
    /// 30 fps.
    pub const FPS_30: Self = Self { num: 30, den: 1 };
    /// 48 fps.
    pub const FPS_48: Self = Self { num: 48, den: 1 };
    /// 50 fps.
    pub const FPS_50: Self = Self { num: 50, den: 1 };
    /// 59.94 fps (`60000/1001`).
    pub const FPS_59_94: Self = Self {
        num: 60_000,
        den: 1_001,
    };
    /// 60 fps.
    pub const FPS_60: Self = Self { num: 60, den: 1 };
    /// 119.88 fps (`120000/1001`).
    pub const FPS_119_88: Self = Self {
        num: 120_000,
        den: 1_001,
    };
    /// 120 fps.
    pub const FPS_120: Self = Self { num: 120, den: 1 };

    /// Rates offered in sequence settings.
    pub const COMMON: [Self; 11] = [
        Self::FPS_23_976,
        Self::FPS_24,
        Self::FPS_25,
        Self::FPS_29_97,
        Self::FPS_30,
        Self::FPS_48,
        Self::FPS_50,
        Self::FPS_59_94,
        Self::FPS_60,
        Self::FPS_119_88,
        Self::FPS_120,
    ];

    /// Construct a rate from a positive numerator and denominator, reduced to
    /// lowest terms.
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::InvalidRate`] unless both parts are positive.
    pub fn new(num: i64, den: i64) -> Result<Self, TimeError> {
        if num <= 0 || den <= 0 {
            return Err(TimeError::InvalidRate);
        }
        let divisor = i64::try_from(gcd(num.unsigned_abs(), den.unsigned_abs()))
            .map_err(|_| TimeError::Overflow)?;
        Ok(Self {
            num: num / divisor,
            den: den / divisor,
        })
    }

    /// Numerator in frames per second.
    pub fn num(self) -> i64 {
        self.num
    }

    /// Denominator in frames per second.
    pub fn den(self) -> i64 {
        self.den
    }

    /// This rate in frames per second.
    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// A compact label such as `24` or `23.976`.
    pub fn label(self) -> String {
        if self.den == 1 {
            return self.num.to_string();
        }
        let value = format!("{:.3}", self.as_f64());
        value.trim_end_matches('0').trim_end_matches('.').to_owned()
    }

    /// Duration of one frame in ticks, floored when the rate is exotic.
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::Overflow`] or [`TimeError::OutOfRange`] when the
    /// duration cannot be represented.
    pub fn frame_duration(self) -> Result<Tick, TimeError> {
        let numerator = i128::from(TICKS_PER_SECOND)
            .checked_mul(i128::from(self.den))
            .ok_or(TimeError::Overflow)?;
        let value = divide_rounded(numerator, i128::from(self.num), Rounding::Floor)?;
        i64::try_from(value)
            .map(Tick)
            .map_err(|_| TimeError::OutOfRange)
    }

    /// Index of the frame containing `time`, floored toward negative infinity.
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::Overflow`] or [`TimeError::OutOfRange`] when the
    /// frame index cannot be represented.
    pub fn frame_at(self, time: Tick) -> Result<i64, TimeError> {
        let numerator = i128::from(time.0)
            .checked_mul(i128::from(self.num))
            .ok_or(TimeError::Overflow)?;
        let denominator = i128::from(TICKS_PER_SECOND)
            .checked_mul(i128::from(self.den))
            .ok_or(TimeError::Overflow)?;
        let value = divide_rounded(numerator, denominator, Rounding::Floor)?;
        i64::try_from(value).map_err(|_| TimeError::OutOfRange)
    }

    /// Start tick of frame `frame`, rounded up when the boundary falls between
    /// ticks so that [`FrameRate::frame_at`] returns the same frame.
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::Overflow`] or [`TimeError::OutOfRange`] when the
    /// boundary cannot be represented.
    pub fn tick_of(self, frame: i64) -> Result<Tick, TimeError> {
        let numerator = i128::from(frame)
            .checked_mul(i128::from(TICKS_PER_SECOND))
            .and_then(|value| value.checked_mul(i128::from(self.den)))
            .ok_or(TimeError::Overflow)?;
        let denominator = i128::from(self.num);
        let boundary = divide_rounded(numerator, denominator, Rounding::Ceil)?;
        i64::try_from(boundary)
            .map(Tick)
            .map_err(|_| TimeError::OutOfRange)
    }

    /// Snap `time` down to a frame boundary.
    ///
    /// # Errors
    ///
    /// Propagates [`FrameRate::frame_at`] and [`FrameRate::tick_of`] failures.
    pub fn snap(self, time: Tick) -> Result<Tick, TimeError> {
        self.tick_of(self.frame_at(time)?)
    }

    /// Snap `time` to the nearest frame boundary, with ties toward the later
    /// boundary.
    ///
    /// # Errors
    ///
    /// Propagates [`FrameRate::frame_at`] and [`FrameRate::tick_of`] failures.
    pub fn snap_nearest(self, time: Tick) -> Result<Tick, TimeError> {
        let frame = self.frame_at(time)?;
        let before = self.tick_of(frame)?;
        if before == time {
            return Ok(before);
        }
        let after_frame = frame.checked_add(1).ok_or(TimeError::Overflow)?;
        let after = self.tick_of(after_frame)?;
        let time = i128::from(time.0);
        Ok(
            if time - i128::from(before.0) < i128::from(after.0) - time {
                before
            } else {
                after
            },
        )
    }
}

impl fmt::Display for FrameRate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} fps", self.label())
    }
}

/// A nonempty half-open range `[start, end)` in ticks.
///
/// Empty selections are represented by absence (`Option::None`), never by an
/// inverted or zero-length range. Use [`SourceRange`] when the coordinates refer
/// to source media.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "TimeRangeWire")]
pub struct TimeRange {
    start: Tick,
    end: Tick,
}

impl TimeRange {
    /// Construct a range from half-open bounds.
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::EmptyRange`] unless `end > start` and
    /// [`TimeError::Overflow`] when the duration does not fit `i64`.
    pub fn new(start: Tick, end: Tick) -> Result<Self, TimeError> {
        if end <= start {
            return Err(TimeError::EmptyRange);
        }
        end.checked_sub(start).ok_or(TimeError::Overflow)?;
        Ok(Self { start, end })
    }

    /// Construct a range from a start and a positive duration.
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::EmptyRange`] when `duration` is not positive and
    /// [`TimeError::Overflow`] when the end does not fit `i64`.
    pub fn from_start_duration(start: Tick, duration: Tick) -> Result<Self, TimeError> {
        if duration <= Tick::ZERO {
            return Err(TimeError::EmptyRange);
        }
        let end = start.checked_add(duration).ok_or(TimeError::Overflow)?;
        Self::new(start, end)
    }

    /// First tick in the range.
    pub fn start(self) -> Tick {
        self.start
    }

    /// First tick after the range.
    pub fn end(self) -> Tick {
        self.end
    }

    /// Number of ticks in the range.
    pub fn duration(self) -> Tick {
        Tick(self.end.0.saturating_sub(self.start.0))
    }

    /// Whether `time` lies in `[start, end)`.
    pub fn contains(self, time: Tick) -> bool {
        self.start <= time && time < self.end
    }

    /// Whether two ranges share at least one tick.
    pub fn overlaps(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }

    /// The shared range, or `None` when the ranges only touch or are disjoint.
    pub fn intersect(self, other: Self) -> Option<Self> {
        let start = self.start.max(other.start);
        let end = self.end.min(other.end);
        if end > start {
            Self::new(start, end).ok()
        } else {
            None
        }
    }
}

/// A nonempty half-open range in source-media ticks.
pub type SourceRange = TimeRange;

/// Divide after normalizing a negative denominator, applying `rounding`.
fn divide_rounded(
    numerator: i128,
    denominator: i128,
    rounding: Rounding,
) -> Result<i128, TimeError> {
    if denominator == 0 {
        return Err(TimeError::InvalidRate);
    }
    let (numerator, denominator) = if denominator < 0 {
        (
            numerator.checked_neg().ok_or(TimeError::Overflow)?,
            denominator.checked_neg().ok_or(TimeError::Overflow)?,
        )
    } else {
        (numerator, denominator)
    };
    let quotient = numerator.div_euclid(denominator);
    let remainder = numerator.rem_euclid(denominator);
    match rounding {
        Rounding::Floor => Ok(quotient),
        Rounding::Ceil if remainder != 0 => quotient.checked_add(1).ok_or(TimeError::Overflow),
        Rounding::Ceil => Ok(quotient),
        Rounding::Nearest => {
            let doubled = remainder.checked_mul(2).ok_or(TimeError::Overflow)?;
            if doubled < denominator || (doubled == denominator && numerator < 0) {
                Ok(quotient)
            } else {
                quotient.checked_add(1).ok_or(TimeError::Overflow)
            }
        }
    }
}

fn gcd(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

const I64_MIN_AS_F64: f64 = -9_223_372_036_854_775_808.0;
const I64_MAX_EXCLUSIVE_AS_F64: f64 = 9_223_372_036_854_775_808.0;

#[derive(Deserialize, schemars::JsonSchema)]
struct FrameRateWire {
    num: i64,
    den: i64,
}

impl TryFrom<FrameRateWire> for FrameRate {
    type Error = TimeError;

    fn try_from(wire: FrameRateWire) -> Result<Self, Self::Error> {
        Self::new(wire.num, wire.den)
    }
}

#[derive(Deserialize, schemars::JsonSchema)]
struct TimeRangeWire {
    start: Tick,
    end: Tick,
}

impl TryFrom<TimeRangeWire> for TimeRange {
    type Error = TimeError;

    fn try_from(wire: TimeRangeWire) -> Result<Self, Self::Error> {
        Self::new(wire.start, wire.end)
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameRate, Rounding, SourceRange, TICKS_PER_SECOND, Tick, TimeError, TimeRange};
    use proptest::prelude::*;

    const RATES: [FrameRate; 9] = [
        FrameRate::FPS_23_976,
        FrameRate::FPS_24,
        FrameRate::FPS_25,
        FrameRate::FPS_29_97,
        FrameRate::FPS_30,
        FrameRate::FPS_50,
        FrameRate::FPS_59_94,
        FrameRate::FPS_60,
        FrameRate::FPS_120,
    ];

    fn ten_hour_frame_limit(rate: FrameRate) -> i64 {
        36_000 * rate.num() / rate.den()
    }

    fn assert_frame_roundtrip(rate: FrameRate, frame: i64) {
        let tick = rate.tick_of(frame).unwrap();
        assert_eq!(rate.frame_at(tick).unwrap(), frame, "{rate} frame {frame}");
        let next = rate.tick_of(frame + 1).unwrap();
        assert_eq!(
            rate.frame_at(next - Tick(1)).unwrap(),
            frame,
            "{rate} last tick of frame {frame}"
        );
        assert_eq!(rate.snap(tick).unwrap(), tick);
    }

    #[test]
    fn frame_tick_roundtrip_is_exact_over_ten_hours() {
        for rate in RATES {
            let limit = ten_hour_frame_limit(rate);
            let stride = (limit / 50_000).max(1) as usize;
            let mut sampled = 0_u32;
            for frame in (0..=limit).step_by(stride) {
                assert_frame_roundtrip(rate, frame);
                sampled += 1;
            }
            for frame in [0, 1, limit - 1, limit] {
                assert_frame_roundtrip(rate, frame);
            }
            assert!(sampled >= 40_000, "{rate}: only {sampled} frames sampled");
        }
    }

    #[test]
    fn sample_tick_conversions_are_exact() {
        for rate in [44_100, 48_000, 96_000] {
            for sample in 0..=rate * 10 {
                let tick = Tick::from_units(sample, rate, Rounding::Floor).unwrap();
                assert_eq!(
                    tick.to_units(rate, Rounding::Floor).unwrap(),
                    sample,
                    "{rate} Hz sample {sample}"
                );
            }
            let ten_hours = rate * 36_000;
            for sample in [ten_hours - 1, ten_hours, ten_hours + 1] {
                for rounding in [Rounding::Floor, Rounding::Ceil, Rounding::Nearest] {
                    let tick = Tick::from_units(sample, rate, rounding).unwrap();
                    assert_eq!(tick.to_units(rate, rounding).unwrap(), sample);
                }
            }
        }
    }

    #[test]
    fn checked_arithmetic_reports_overflow_without_panicking() {
        assert_eq!(Tick(i64::MAX).checked_add(Tick(1)), None);
        assert_eq!(Tick(i64::MIN).checked_sub(Tick(1)), None);
        assert_eq!(Tick(i64::MIN).checked_neg(), None);
        assert_eq!(Tick(i64::MAX).saturating_add(Tick(1)), Tick(i64::MAX));
        assert_eq!(Tick(i64::MIN).saturating_sub(Tick(1)), Tick(i64::MIN));
        assert_eq!(Tick(i64::MIN).saturating_neg(), Tick(i64::MAX));
        assert_eq!(Tick(-5).abs(), Tick(5));
        assert_eq!(Tick(i64::MIN).abs(), Tick(i64::MAX));
        assert_eq!(Tick(3).clamp(Tick(10), Tick(0)), Tick(0));
    }

    #[test]
    fn rational_conversions_reject_invalid_rates_and_overflow() {
        assert_eq!(
            Tick::from_rational(1, 1, 0, Rounding::Floor),
            Err(TimeError::InvalidRate)
        );
        assert_eq!(
            Tick::from_rational(1, -1, 1_000, Rounding::Floor),
            Err(TimeError::InvalidRate)
        );
        assert_eq!(
            Tick::from_rational(1, 1, -1_000, Rounding::Floor),
            Err(TimeError::InvalidRate)
        );
        assert_eq!(
            Tick(1).to_rational(-1, 1_000, Rounding::Floor),
            Err(TimeError::InvalidRate)
        );
        assert_eq!(
            Tick(1).to_rational(1, -1_000, Rounding::Floor),
            Err(TimeError::InvalidRate)
        );
        assert_eq!(
            Tick::from_units(1, 0, Rounding::Floor),
            Err(TimeError::InvalidRate)
        );
        assert_eq!(
            Tick::from_units(1, -48_000, Rounding::Floor),
            Err(TimeError::InvalidRate)
        );
        assert_eq!(
            Tick(1).to_units(0, Rounding::Floor),
            Err(TimeError::InvalidRate)
        );
        assert_eq!(
            Tick(1).to_rational(0, 1, Rounding::Floor),
            Err(TimeError::InvalidRate)
        );
        assert_eq!(
            Tick::from_rational(i64::MAX, i64::MAX, 1, Rounding::Floor),
            Err(TimeError::Overflow)
        );
        assert_eq!(
            Tick::from_rational(i64::MAX, 1, 1, Rounding::Floor),
            Err(TimeError::OutOfRange)
        );
        assert_eq!(
            Tick(i64::MAX).to_rational(1, i64::MAX, Rounding::Floor),
            Err(TimeError::OutOfRange)
        );
        assert_eq!(
            Tick(i64::MAX).to_units(i64::MAX, Rounding::Floor),
            Err(TimeError::OutOfRange)
        );
    }

    #[test]
    fn seconds_conversion_rejects_non_finite_and_out_of_range() {
        assert_eq!(
            Tick::from_seconds_f64(f64::NAN, Rounding::Nearest),
            Err(TimeError::NonFiniteSeconds)
        );
        assert_eq!(
            Tick::from_seconds_f64(f64::INFINITY, Rounding::Nearest),
            Err(TimeError::NonFiniteSeconds)
        );
        assert_eq!(
            Tick::from_seconds_f64(f64::NEG_INFINITY, Rounding::Floor),
            Err(TimeError::NonFiniteSeconds)
        );
        assert_eq!(
            Tick::from_seconds_f64(1.0e300, Rounding::Nearest),
            Err(TimeError::OutOfRange)
        );
        assert_eq!(
            Tick::from_seconds_f64(-1.0e300, Rounding::Nearest),
            Err(TimeError::OutOfRange)
        );
        assert_eq!(
            Tick::from_seconds_f64(1.0, Rounding::Floor).unwrap(),
            Tick(TICKS_PER_SECOND)
        );
        assert_eq!(
            Tick::from_seconds_f64(-1.0, Rounding::Ceil).unwrap(),
            Tick(-TICKS_PER_SECOND)
        );
        assert_eq!(Tick(TICKS_PER_SECOND).seconds(), 1.0);
    }

    #[test]
    fn rational_rounding_is_explicit_at_ties() {
        let denominator = 2 * TICKS_PER_SECOND;
        assert_eq!(
            Tick::from_rational(3, 1, denominator, Rounding::Floor).unwrap(),
            Tick(1)
        );
        assert_eq!(
            Tick::from_rational(3, 1, denominator, Rounding::Ceil).unwrap(),
            Tick(2)
        );
        assert_eq!(
            Tick::from_rational(3, 1, denominator, Rounding::Nearest).unwrap(),
            Tick(2)
        );
        assert_eq!(
            Tick::from_rational(-3, 1, denominator, Rounding::Floor).unwrap(),
            Tick(-2)
        );
        assert_eq!(
            Tick::from_rational(-3, 1, denominator, Rounding::Ceil).unwrap(),
            Tick(-1)
        );
        assert_eq!(
            Tick::from_rational(-3, 1, denominator, Rounding::Nearest).unwrap(),
            Tick(-2)
        );
        assert_eq!(
            Tick::from_rational(-1, 1, denominator, Rounding::Nearest).unwrap(),
            Tick(-1)
        );
    }

    #[test]
    fn common_frame_rates_have_exact_frame_durations() {
        for rate in FrameRate::COMMON {
            let duration = rate.frame_duration().unwrap();
            assert_eq!(
                i128::from(duration.0) * i128::from(rate.num()),
                i128::from(TICKS_PER_SECOND) * i128::from(rate.den()),
                "{rate}"
            );
        }
    }

    #[test]
    fn frame_rate_constructor_rejects_invalid_rates_and_normalizes() {
        assert_eq!(FrameRate::new(48_000, 2_000).unwrap(), FrameRate::FPS_24);
        assert_eq!(FrameRate::new(60_000, 1_001).unwrap(), FrameRate::FPS_59_94);
        assert_eq!(FrameRate::new(0, 1), Err(TimeError::InvalidRate));
        assert_eq!(FrameRate::new(1, 0), Err(TimeError::InvalidRate));
        assert_eq!(FrameRate::new(-24, 1), Err(TimeError::InvalidRate));
        assert_eq!(FrameRate::new(24, -1), Err(TimeError::InvalidRate));
        for (less, greater) in [
            (FrameRate::FPS_29_97, FrameRate::FPS_30),
            (
                FrameRate::new(i64::MAX - 1, i64::MAX).unwrap(),
                FrameRate::new(i64::MAX, i64::MAX - 1).unwrap(),
            ),
        ] {
            assert!(less < greater);
            assert!(greater > less);
        }
        assert_eq!(FrameRate::default(), FrameRate::FPS_23_976);
        assert_eq!(FrameRate::FPS_23_976.label(), "23.976");
        assert_eq!(FrameRate::FPS_24.label(), "24");
    }

    #[test]
    fn nearest_frame_ties_choose_the_later_boundary() {
        let rate = FrameRate::FPS_25;
        let duration = rate.frame_duration().unwrap().0;
        for frame in [-2, -1, 0, 1] {
            let start = rate.tick_of(frame).unwrap();
            let midpoint = start.checked_add(Tick(duration / 2)).unwrap();
            assert_eq!(
                rate.snap_nearest(midpoint).unwrap(),
                rate.tick_of(frame + 1).unwrap()
            );
            assert_eq!(rate.snap_nearest(midpoint - Tick(1)).unwrap(), start);
        }
    }

    #[test]
    fn exact_frame_snap_does_not_require_a_later_representable_frame() {
        let one_tick = FrameRate::new(TICKS_PER_SECOND, 1).unwrap();
        for time in [
            Tick(i64::MIN),
            Tick(-1),
            Tick::ZERO,
            Tick(1),
            Tick(i64::MAX),
        ] {
            assert_eq!(one_tick.snap_nearest(time), Ok(time));
        }
        let rate = FrameRate::FPS_25;
        let last_frame = rate.frame_at(Tick(i64::MAX)).unwrap();
        let last_start = rate.tick_of(last_frame).unwrap();
        assert_eq!(rate.snap_nearest(last_start), Ok(last_start));
    }

    #[test]
    fn ranges_are_half_open_and_nonempty() {
        let range = TimeRange::new(Tick(10), Tick(20)).unwrap();
        assert_eq!(range.start(), Tick(10));
        assert_eq!(range.end(), Tick(20));
        assert_eq!(range.duration(), Tick(10));
        assert!(range.contains(Tick(10)));
        assert!(range.contains(Tick(19)));
        assert!(!range.contains(Tick(20)));
        assert_eq!(
            range.intersect(TimeRange::new(Tick(19), Tick(30)).unwrap()),
            Some(TimeRange::new(Tick(19), Tick(20)).unwrap())
        );
        assert_eq!(
            range.intersect(TimeRange::new(Tick(20), Tick(30)).unwrap()),
            None
        );
        assert!(!range.overlaps(TimeRange::new(Tick(20), Tick(30)).unwrap()));
        assert!(range.overlaps(TimeRange::new(Tick(19), Tick(30)).unwrap()));
        assert_eq!(
            TimeRange::new(Tick(20), Tick(10)),
            Err(TimeError::EmptyRange)
        );
        assert_eq!(
            TimeRange::new(Tick(10), Tick(10)),
            Err(TimeError::EmptyRange)
        );
        assert_eq!(
            TimeRange::new(Tick(i64::MIN), Tick(i64::MAX)),
            Err(TimeError::Overflow)
        );
        let from_duration = TimeRange::from_start_duration(Tick(5), Tick(3)).unwrap();
        assert_eq!(from_duration, TimeRange::new(Tick(5), Tick(8)).unwrap());
        assert_eq!(
            TimeRange::from_start_duration(Tick(5), Tick(0)),
            Err(TimeError::EmptyRange)
        );
        let source: SourceRange = range;
        assert_eq!(source, range);
    }

    #[test]
    fn tick_scale_rounding_ties_overflow_and_invalid_ratios() {
        for (tick, expected) in [(1, 1), (-1, -1), (3, 2), (-3, -2)] {
            assert_eq!(
                Tick(tick).checked_scale(1, 2, Rounding::Nearest).unwrap(),
                Tick(expected)
            );
        }
        assert_eq!(
            Tick(i64::MIN)
                .checked_scale(1, 1, Rounding::Nearest)
                .unwrap(),
            Tick(i64::MIN)
        );
        assert_eq!(
            Tick(i64::MAX)
                .checked_scale(1, 1, Rounding::Nearest)
                .unwrap(),
            Tick(i64::MAX)
        );
        assert!(
            Tick(i64::MAX)
                .checked_scale(2, 1, Rounding::Nearest)
                .is_err()
        );
        assert!(Tick(0).checked_scale(1, 0, Rounding::Nearest).is_err());
        assert!(Tick(0).checked_scale(-1, 1, Rounding::Nearest).is_err());
    }

    proptest! {
        #[test]
        fn sampled_frame_tick_roundtrip_has_no_drift(
            frame in 0_i64..=4_320_000,
            index in 0_usize..RATES.len(),
        ) {
            let rate = RATES[index];
            let limit = ten_hour_frame_limit(rate);
            let frame = frame % (limit + 1);
            assert_frame_roundtrip(rate, frame);
        }

        #[test]
        fn extreme_rates_remain_monotonic(
            num in 1_i64..=i64::MAX,
            den in 1_i64..=i64::MAX,
            left in any::<i64>(),
            right in any::<i64>(),
        ) {
            let rate = FrameRate::new(num, den).unwrap();
            let (low, high) = (left.min(right), left.max(right));
            if let (Ok(low_tick), Ok(high_tick)) = (rate.tick_of(low), rate.tick_of(high)) {
                prop_assert!(low_tick <= high_tick);
            }
            if let (Ok(low_frame), Ok(high_frame)) =
                (rate.frame_at(Tick(low)), rate.frame_at(Tick(high)))
            {
                prop_assert!(low_frame <= high_frame);
            }
            let _ = rate.frame_duration();
            let _ = rate.snap_nearest(Tick(left));
        }

        #[test]
        fn rational_rounding_stays_within_one_tick(
            numerator in -1_000_000_i64..=1_000_000,
            denominator in 1_i64..=1_000_000,
        ) {
            let floor = Tick::from_rational(numerator, 1, denominator, Rounding::Floor).unwrap();
            let ceil = Tick::from_rational(numerator, 1, denominator, Rounding::Ceil).unwrap();
            let nearest =
                Tick::from_rational(numerator, 1, denominator, Rounding::Nearest).unwrap();
            prop_assert!(floor <= nearest);
            prop_assert!(nearest <= ceil);
            prop_assert!(ceil - floor <= Tick(1));
        }
    }
}
