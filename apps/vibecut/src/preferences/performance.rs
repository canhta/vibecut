//! Project-default frame rate, hardware acceleration and the aggregate cache
//! byte limit. Values are validated on load and set; the limit and the
//! acceleration choice apply when the app next starts.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};

/// Frame rates the project settings offer (CapCut's frame-rate select).
pub const PROJECT_FPS: [u32; 5] = [24, 25, 30, 60, 120];
/// Matches the frame rate new projects already get from `ProjectSettings`.
pub use crate::services::lifecycle::DEFAULT_PROJECT_FPS;
/// Smallest aggregate cache budget the settings dialog accepts.
pub const MIN_CACHE_LIMIT: u64 = 256 << 20;
/// Largest aggregate cache budget the settings dialog accepts.
pub const MAX_CACHE_LIMIT: u64 = 8 << 30;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum HardwareAcceleration {
    #[default]
    Auto,
    On,
    Off,
}

impl HardwareAcceleration {
    /// The export encoder this choice selects. `Auto` keeps the media crate's
    /// per-platform default; decoding is not covered by this preference.
    pub fn encode_preference(self) -> media::EncodePreference {
        match self {
            Self::Auto => media::EncodePreference::default(),
            Self::On => media::EncodePreference::Hardware,
            Self::Off => media::EncodePreference::Software,
        }
    }
}

pub fn validate_project_fps(fps: u32) -> Result<u32> {
    if PROJECT_FPS.contains(&fps) {
        Ok(fps)
    } else {
        Err(Error::Invalid("default frame rate"))
    }
}

pub fn validate_cache_limit(bytes: u64) -> Result<u64> {
    if (MIN_CACHE_LIMIT..=MAX_CACHE_LIMIT).contains(&bytes) {
        Ok(bytes)
    } else {
        Err(Error::Invalid("cache limit"))
    }
}
