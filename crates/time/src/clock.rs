//! Formats media time as fixed display clocks for panels and project lists.

use crate::{TICKS_PER_SECOND, Tick};

const TICKS_PER_MILLISECOND: i64 = TICKS_PER_SECOND / 1_000;

/// Format `HH:MM:SS.mmm` with whole milliseconds floored from exact ticks.
/// Negative times read as zero.
pub fn format_clock(time: Tick) -> String {
    let millis = time.0.max(0) / TICKS_PER_MILLISECOND;
    let hours = millis / 3_600_000;
    let minutes = millis / 60_000 % 60;
    let seconds = millis / 1_000 % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{:03}", millis % 1_000)
}

/// Format `M:SS` with total minutes, so one hour reads as `60:00`. Whole seconds
/// are floored; negative times read as zero.
pub fn format_short_clock(time: Tick) -> String {
    let seconds = time.0.max(0) / TICKS_PER_SECOND;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// Format `M:SS` below one hour and `H:MM:SS` from one hour. Whole seconds are
/// floored; negative times read as zero.
pub fn format_duration(time: Tick) -> String {
    let seconds = time.0.max(0) / TICKS_PER_SECOND;
    let (hours, minutes, seconds) = (seconds / 3_600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn millis(milliseconds: i64) -> Tick {
        Tick(milliseconds * TICKS_PER_MILLISECOND)
    }

    #[test]
    fn display_clocks_follow_fixed_rows() {
        for (time, clock, short, duration) in [
            (Tick::ZERO, "00:00:00.000", "0:00", "00:00"),
            (Tick(-1), "00:00:00.000", "0:00", "00:00"),
            (Tick(TICKS_PER_MILLISECOND - 1), "00:00:00.000", "0:00", "00:00"),
            (millis(1), "00:00:00.001", "0:00", "00:00"),
            (millis(1_500), "00:00:01.500", "0:01", "00:01"),
            (millis(65_900), "00:01:05.900", "1:05", "01:05"),
            (millis(3_599_999), "00:59:59.999", "59:59", "59:59"),
            (millis(3_600_000), "01:00:00.000", "60:00", "1:00:00"),
            (millis(3_661_250), "01:01:01.250", "61:01", "1:01:01"),
        ] {
            assert_eq!(format_clock(time), clock, "{time:?}");
            assert_eq!(format_short_clock(time), short, "{time:?}");
            assert_eq!(format_duration(time), duration, "{time:?}");
        }
    }
}
