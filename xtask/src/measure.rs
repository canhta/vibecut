//! Local measurement harnesses: media throughput, clip labels and UI comparison.
mod benchmark;
mod clip_eval;
mod ui_compare;

pub(crate) use benchmark::media;
pub(crate) use clip_eval::run as clip_eval;
pub(crate) use ui_compare::run as ui_compare;
