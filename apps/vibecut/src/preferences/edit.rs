//! The Settings Edit tab's choices: the many-step sizes, the still-image length and its
//! unit, the loudness preset, the time code format, audio scrubbing and the export chime.
//! Each is read by the feature that owns the behaviour; this module holds the defaults,
//! the bounds and the one timecode label formatter.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use time::{FrameRate, Tick, TimecodeStyle};

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

/// The Target loudness select's three options, in the reference order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoudnessPreset {
    /// Default (-23 LUFS), the reference default.
    #[default]
    Standard,
    /// YouTube (-14 LUFS).
    Youtube,
    /// TikTok (-12 LUFS).
    Tiktok,
}

impl LoudnessPreset {
    pub const ALL: [Self; 3] = [Self::Standard, Self::Youtube, Self::Tiktok];

    /// Integrated loudness a new normalize target starts at.
    pub fn lufs(self) -> i32 {
        match self {
            Self::Standard => -23,
            Self::Youtube => -14,
            Self::Tiktok => -12,
        }
    }
}

/// The unit the Image duration field shows; the stored length stays in ms.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageDurationUnit {
    /// Seconds, the reference default.
    #[default]
    Seconds,
    /// Frames at the default project frame rate.
    Frames,
}

impl ImageDurationUnit {
    pub const ALL: [Self; 2] = [Self::Seconds, Self::Frames];
}

/// The Time code select's two options, in the reference order. Every timecode label in
/// the app is written by [`TimeCode::format`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeCode {
    /// HH:MM:SS+frame, the reference default; drop-frame rates mark the frame with `;`.
    #[default]
    SecondsPlusFrame,
    /// HH:MM:SS:FF: colons for every rate.
    AllColons,
}

impl TimeCode {
    pub const ALL: [Self; 2] = [Self::SecondsPlusFrame, Self::AllColons];

    pub fn style(self) -> TimecodeStyle {
        match self {
            Self::SecondsPlusFrame => TimecodeStyle::SecondsPlusFrame,
            Self::AllColons => TimecodeStyle::AllColons,
        }
    }

    /// The timecode label for `at`, or the exact clock when `rate` has no timecode.
    pub fn format(self, at: Tick, rate: FrameRate) -> String {
        time::format_timecode_styled(at, rate, self.style())
            .unwrap_or_else(|_| time::format_clock(at))
    }

    /// The tick a typed label names; accepts exactly what [`TimeCode::format`] writes.
    pub fn parse(self, label: &str, rate: FrameRate) -> std::result::Result<Tick, time::TimeError> {
        time::parse_timecode_styled(label, rate, self.style())
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_code_options_share_one_formatter_that_falls_back_to_the_clock() {
        let df = FrameRate::FPS_29_97;
        let at = df.tick_of(1800).unwrap();
        assert_eq!(TimeCode::SecondsPlusFrame.format(at, df), "00:01:00;02");
        assert_eq!(TimeCode::AllColons.format(at, df), "00:01:00:02");
        for code in TimeCode::ALL {
            let label = code.format(at, df);
            assert_eq!(code.parse(&label, df), Ok(at));
        }
        let fps = FrameRate::FPS_30;
        let at = fps.tick_of(45).unwrap();
        for code in TimeCode::ALL {
            assert_eq!(code.format(at, fps), "00:00:01:15");
        }
        // 120 fps has no timecode: the exact clock stands in for every label.
        let unsupported = FrameRate::new(120, 1).unwrap();
        let at = Tick(time::TICKS_PER_SECOND);
        assert_eq!(
            TimeCode::AllColons.format(at, unsupported),
            time::format_clock(at)
        );
    }
}
