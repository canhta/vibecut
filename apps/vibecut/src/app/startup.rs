//! What a start offers once: a newer public release or the last run's crash report.
mod crash;
mod notice;
mod update;

pub use crash::{install as install_crash_log, save as save_crash_report};
pub(super) use notice::Notices;
