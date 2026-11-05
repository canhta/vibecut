//! Constant playback speed and its exact source/output duration mapping.

use crate::{Rounding, Tick, TimeError};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Constant playback speed on a 0.001 grid, from 0.01x to 5x.
///
/// Speed is stored as integer thousandths, so every admitted value is an exact
/// rational and no timeline duration is ever accumulated in floating point. A
/// source span of `s` ticks plays for `floor(s / speed)` output ticks; the
/// floor keeps adjacent pieces of a split or trim from overlapping their
/// neighbour. JSON carries the decimal value (`1.5`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Speed(u16);

impl Speed {
    /// Normal speed (1x).
    pub const UNITY: Self = Self(1000);
    /// Slowest admitted speed (0.01x), in thousandths.
    pub const MIN_THOUSANDTHS: u16 = 10;
    /// Fastest admitted speed (5x), in thousandths.
    pub const MAX_THOUSANDTHS: u16 = 5000;

    /// Construct from integer thousandths of normal speed.
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::InvalidSpeed`] outside 10..=5000.
    pub fn from_thousandths(thousandths: u16) -> Result<Self, TimeError> {
        if (Self::MIN_THOUSANDTHS..=Self::MAX_THOUSANDTHS).contains(&thousandths) {
            Ok(Self(thousandths))
        } else {
            Err(TimeError::InvalidSpeed)
        }
    }

    /// Convert a decimal speed once at an input boundary, rounding to the
    /// nearest thousandth (ties away from zero).
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::InvalidSpeed`] for NaN, infinity or a value outside
    /// 0.01..=5 after rounding.
    pub fn from_f64(speed: f64) -> Result<Self, TimeError> {
        if !speed.is_finite() {
            return Err(TimeError::InvalidSpeed);
        }
        let scaled = (speed * 1000.0).round();
        if !(f64::from(Self::MIN_THOUSANDTHS)..=f64::from(Self::MAX_THOUSANDTHS)).contains(&scaled)
        {
            return Err(TimeError::InvalidSpeed);
        }
        Self::from_thousandths(scaled as u16)
    }

    /// Integer thousandths of normal speed.
    pub fn thousandths(self) -> u16 {
        self.0
    }

    /// Decimal value for display and JSON.
    pub fn as_f64(self) -> f64 {
        f64::from(self.0) / 1000.0
    }

    /// Whether this is exactly 1x.
    pub fn is_unity(self) -> bool {
        self == Self::UNITY
    }

    /// The reduced ratio `(num, den)` with `speed = num / den`.
    pub fn ratio(self) -> (i64, i64) {
        let divisor = i64::from(crate::gcd(u64::from(self.0), 1000) as u16);
        (i64::from(self.0) / divisor, 1000 / divisor)
    }

    /// Output ticks that `source` ticks play for: `floor(source / speed)`.
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::OutOfRange`] when the result does not fit `i64`.
    pub fn output_duration(self, source: Tick) -> Result<Tick, TimeError> {
        let (num, den) = self.ratio();
        source.checked_scale(den, num, Rounding::Floor)
    }

    /// Source ticks consumed after `output` ticks of playback, with an explicit
    /// rounding mode.
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::OutOfRange`] when the result does not fit `i64`.
    pub fn source_offset(self, output: Tick, rounding: Rounding) -> Result<Tick, TimeError> {
        let (num, den) = self.ratio();
        output.checked_scale(num, den, rounding)
    }

    /// Output ticks at which `source` ticks of progress are reached, with an
    /// explicit rounding mode.
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::OutOfRange`] when the result does not fit `i64`.
    pub fn output_offset(self, source: Tick, rounding: Rounding) -> Result<Tick, TimeError> {
        let (num, den) = self.ratio();
        source.checked_scale(den, num, rounding)
    }

    /// The source span that plays for `output` ticks without exceeding them. At
    /// speeds of 1x and above it is exactly `ceil(output * speed)`, the first
    /// source tick the next output tick shows; below 1x, where whole source ticks
    /// are longer than an output tick, it is the longest span that still fits.
    /// A trim or split uses it to keep an edge at or inside the requested time.
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::OutOfRange`] when the result does not fit `i64`.
    pub fn source_within(self, output: Tick) -> Result<Tick, TimeError> {
        let (num, den) = self.ratio();
        // Largest s with floor(s * den / num) <= output is ceil((output + 1) * num / den) - 1.
        let next = i128::from(output.0)
            .checked_add(1)
            .and_then(|value| value.checked_mul(i128::from(num)))
            .ok_or(TimeError::Overflow)?;
        let den = i128::from(den);
        let longest = (next + den - 1).div_euclid(den) - 1;
        let exact = self.source_offset(output, Rounding::Ceil)?;
        let span = longest.min(i128::from(exact.0));
        i64::try_from(span)
            .map(Tick)
            .map_err(|_| TimeError::OutOfRange)
    }

    /// The reduced factor `(num, den)` that stretches output time when changing
    /// from `self` to `target`: durations multiply by `self / target`.
    pub fn stretch_to(self, target: Self) -> (i64, i64) {
        let divisor = crate::gcd(u64::from(self.0), u64::from(target.0)) as i64;
        (i64::from(self.0) / divisor, i64::from(target.0) / divisor)
    }
}

impl Default for Speed {
    fn default() -> Self {
        Self::UNITY
    }
}

impl Serialize for Speed {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_f64(self.as_f64())
    }
}

impl<'de> Deserialize<'de> for Speed {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = f64::deserialize(deserializer)?;
        Self::from_f64(value).map_err(serde::de::Error::custom)
    }
}

impl schemars::JsonSchema for Speed {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Speed".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "number",
            "minimum": 0.01,
            "maximum": 5.0,
            "description": "Constant playback speed, rounded to the nearest 0.001."
        })
    }
}

impl std::fmt::Display for Speed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}x", self.as_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn grid_bounds_and_rounding() {
        for (input, expected) in [
            (0.01, Some(10)),
            (0.0104, Some(10)),
            (0.0105, Some(11)),
            (1.0, Some(1000)),
            (5.0, Some(5000)),
            (0.0094, None),
            (5.0006, None),
            (0.0, None),
            (-1.0, None),
            (f64::NAN, None),
            (f64::INFINITY, None),
        ] {
            assert_eq!(
                Speed::from_f64(input).ok().map(Speed::thousandths),
                expected,
                "{input}"
            );
        }
    }

    #[test]
    fn ratio_is_reduced_and_exact() {
        for (thousandths, ratio) in [
            (10, (1, 100)),
            (500, (1, 2)),
            (1000, (1, 1)),
            (1500, (3, 2)),
            (2000, (2, 1)),
            (5000, (5, 1)),
            (333, (333, 1000)),
        ] {
            assert_eq!(Speed::from_thousandths(thousandths).unwrap().ratio(), ratio);
        }
    }

    #[test]
    fn duration_mapping_table() {
        let second = Tick(crate::TICKS_PER_SECOND);
        for (thousandths, output) in [
            (10, 100 * second.0),
            (500, 2 * second.0),
            (1000, second.0),
            (2000, second.0 / 2),
            (5000, second.0 / 5),
        ] {
            let speed = Speed::from_thousandths(thousandths).unwrap();
            assert_eq!(speed.output_duration(second).unwrap(), Tick(output));
            assert_eq!(
                speed
                    .source_offset(Tick(output), Rounding::Nearest)
                    .unwrap(),
                second
            );
        }
    }

    #[test]
    fn json_is_the_decimal_value() {
        let speed: Speed = serde_json::from_str("0.25").unwrap();
        assert_eq!(speed.thousandths(), 250);
        assert_eq!(serde_json::to_string(&speed).unwrap(), "0.25");
        assert!(serde_json::from_str::<Speed>("6").is_err());
        assert!(serde_json::from_str::<Speed>("\"2\"").is_err());
    }

    proptest! {
        #[test]
        fn source_within_fits_and_is_exact_from_unity_up(thousandths in 10u16..=5000, output in 0i64..1_000_000_000_000) {
            let speed = Speed::from_thousandths(thousandths).unwrap();
            let span = speed.source_within(Tick(output)).unwrap();
            prop_assert!(speed.output_duration(span).unwrap() <= Tick(output));
            if thousandths >= 1000 {
                // The edge lands exactly on the requested output tick.
                prop_assert_eq!(speed.output_duration(span).unwrap(), Tick(output));
                prop_assert_eq!(span, speed.source_offset(Tick(output), Rounding::Ceil).unwrap());
            } else {
                // Longest fit: one more source tick would overshoot.
                prop_assert!(speed.output_duration(Tick(span.0 + 1)).unwrap() > Tick(output));
            }
        }

        #[test]
        fn split_pieces_never_outgrow_the_whole(
            thousandths in 10u16..=5000,
            source in 2i64..1_000_000_000_000,
            cut in 1i64..1_000_000_000_000,
        ) {
            let speed = Speed::from_thousandths(thousandths).unwrap();
            let cut = 1 + cut % (source - 1);
            let whole = speed.output_duration(Tick(source)).unwrap();
            let left = speed.output_duration(Tick(cut)).unwrap();
            let right = speed.output_duration(Tick(source - cut)).unwrap();
            prop_assert!(left.0 + right.0 <= whole.0);
            prop_assert!(whole.0 - (left.0 + right.0) <= 1);
        }
    }
}
