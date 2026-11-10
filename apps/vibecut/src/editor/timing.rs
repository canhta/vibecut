//! Formats editor time on the shared tick grid.
use super::Tick;

pub(in crate::editor) fn display_time(at: Tick) -> String {
    let s = at.seconds().max(0.0);
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        (s / 3600.0) as u64,
        (s / 60.0) as u64 % 60,
        s as u64 % 60,
        ((s.fract() * 1000.0).floor()) as u64
    )
}
/// Compact grid badge: total minutes and seconds, matching the reference.
pub(in crate::editor) fn display_short_time(at: Tick) -> String {
    let seconds = at.seconds().max(0.0) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
