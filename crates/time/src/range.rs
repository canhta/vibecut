//! Nonempty half-open ranges in ticks.

use crate::{Tick, TimeError};
use serde::{Deserialize, Serialize};

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
    use super::*;

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
}
