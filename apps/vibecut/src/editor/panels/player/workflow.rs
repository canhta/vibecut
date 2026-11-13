//! Formats the player's timecode labels.

use crate::editor::{FrameRate, Tick};

pub(in crate::editor) fn format_timecode(at: Tick, rate: FrameRate) -> String {
    time::format_timecode(at, rate).unwrap_or_else(|_| time::format_clock(at))
}
