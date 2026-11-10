//! Formats the player's timecode labels.

use crate::editor::{FrameRate, Tick, display_time};

pub(in crate::editor) fn format_timecode(at: Tick, rate: FrameRate) -> String {
    time::format_timecode(at, rate).unwrap_or_else(|_| display_time(at.max(Tick::ZERO)))
}
