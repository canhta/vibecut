//! The Settings Edit tab's choices: the many-step sizes, the still-image length, the
//! normalize target, audio scrubbing and the export chime. Each is read by the feature
//! that owns the behaviour; this module only holds the defaults and the bounds.

use crate::{Error, Result};

/// Frames one "step many" key moves the playhead (reference default 10).
pub const DEFAULT_FRAME_STEP: u32 = 10;
/// Displayed units one "adjust many" step changes a number field by (reference default 10).
pub const DEFAULT_VALUE_STEP: u32 = 10;
/// Both step sizes are whole numbers in this range.
pub const STEP_RANGE: std::ops::RangeInclusive<u32> = 1..=100;
/// Length a still image gets on insert, in ms (reference default 5.0 s).
pub const DEFAULT_IMAGE_DURATION_MS: u32 = 5000;
/// Bounds of the still-image length, 0.1 s to 10 min.
pub const IMAGE_DURATION_RANGE_MS: std::ops::RangeInclusive<u32> = 100..=600_000;
/// Integrated loudness a new normalize target starts at (reference default -23 LUFS).
pub const DEFAULT_LOUDNESS_TARGET_LUFS: i32 = -23;
/// Whole-LUFS bounds, the project target's own range.
pub const LOUDNESS_TARGET_RANGE_LUFS: std::ops::RangeInclusive<i32> = -50..=-5;

pub fn validate_step(step: u32, what: &'static str) -> Result<u32> {
    if STEP_RANGE.contains(&step) {
        Ok(step)
    } else {
        Err(Error::Invalid(what))
    }
}

pub fn validate_image_duration(ms: u32) -> Result<u32> {
    if IMAGE_DURATION_RANGE_MS.contains(&ms) {
        Ok(ms)
    } else {
        Err(Error::Invalid("image duration"))
    }
}

pub fn validate_loudness_target(lufs: i32) -> Result<i32> {
    if LOUDNESS_TARGET_RANGE_LUFS.contains(&lufs) {
        Ok(lufs)
    } else {
        Err(Error::Invalid("loudness target"))
    }
}
