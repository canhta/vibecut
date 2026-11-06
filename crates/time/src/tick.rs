//! Counts media time in exact integer ticks.

use crate::{Rounding, TimeError, divide_rounded};
use serde::{Deserialize, Serialize};
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

const I64_MIN_AS_F64: f64 = -9_223_372_036_854_775_808.0;
const I64_MAX_EXCLUSIVE_AS_F64: f64 = 9_223_372_036_854_775_808.0;

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

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
