//! Timeline row-height and waveform-size choices of the track header menu.

use serde::{Deserialize, Serialize};

/// Row height choice of the track header menu (103106: Short, Default, Tall;
/// Default is the measured pitch).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackHeight {
    Short,
    #[default]
    Default,
    Tall,
}
impl TrackHeight {
    pub const ALL: [Self; 3] = [Self::Short, Self::Default, Self::Tall];
    /// Row scale against the measured default. The reference shows only the
    /// menu, so Short and Tall are [proposed].
    pub const fn scale(self) -> f32 {
        match self {
            Self::Short => 0.75,
            Self::Default => 1.0,
            Self::Tall => 1.5,
        }
    }
}

/// Audio waveform size in percent, as the header menu lists them (103118);
/// 30 is the checked default.
pub const WAVEFORM_SIZES: [u8; 3] = [30, 60, 100];
/// The waveform size a new document starts with.
pub const DEFAULT_WAVEFORM_SIZE: u8 = 30;
