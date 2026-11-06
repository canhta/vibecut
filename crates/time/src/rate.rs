//! Rational frame rates with exact frame and tick conversion.

use crate::{Rounding, TICKS_PER_SECOND, Tick, TimeError, divide_rounded, gcd};
use serde::{Deserialize, Serialize};
use std::fmt;

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

#[cfg(test)]
mod tests {
    use super::*;
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
    }
}
