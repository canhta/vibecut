//! Project canvas, output audio and loudness settings.
use super::item::ItemId;
use serde::{Deserialize, Serialize};
use time::FrameRate;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct Color(pub [u8; 4]);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanvasSize {
    pub width: u32,
    pub height: u32,
}
impl CanvasSize {
    /// Whether both axes are positive and the frame stays inside the canvas area bound.
    pub const fn is_valid(self) -> bool {
        self.width > 0 && self.height > 0 && (self.width as u64) * (self.height as u64) <= 1 << 26
    }
    /// The nearest even width and height, ties up: an odd axis grows by one so
    /// encoders can open the frame. `None` when an odd maximum axis overflows.
    pub const fn rounded_even(self) -> Option<Self> {
        let width = if self.width.is_multiple_of(2) {
            self.width
        } else {
            match self.width.checked_add(1) {
                Some(width) => width,
                None => return None,
            }
        };
        let height = if self.height.is_multiple_of(2) {
            self.height
        } else {
            match self.height.checked_add(1) {
                Some(height) => height,
                None => return None,
            }
        };
        Some(Self { width, height })
    }
}
/// One fixed aspect preset offered by the canvas settings. `Original` resolves
/// to the retained first visual media size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AspectPreset {
    Original,
    #[serde(rename = "16:9")]
    Landscape,
    #[serde(rename = "4:3")]
    Standard,
    #[serde(rename = "2:1")]
    Wide,
    #[serde(rename = "9:16")]
    Portrait,
    #[serde(rename = "1:1")]
    Square,
    #[serde(rename = "3:4")]
    Tall,
}
impl AspectPreset {
    /// Presets in the reference order.
    pub const ALL: [Self; 7] = [
        Self::Original,
        Self::Landscape,
        Self::Standard,
        Self::Wide,
        Self::Portrait,
        Self::Square,
        Self::Tall,
    ];
    /// Long edge 1920; `None` for `Original` until a first visual clip exists.
    pub const fn size(self) -> Option<CanvasSize> {
        Some(match self {
            Self::Original => return None,
            Self::Landscape => CanvasSize {
                width: 1920,
                height: 1080,
            },
            Self::Standard => CanvasSize {
                width: 1440,
                height: 1080,
            },
            Self::Wide => CanvasSize {
                width: 1920,
                height: 960,
            },
            Self::Portrait => CanvasSize {
                width: 1080,
                height: 1920,
            },
            Self::Square => CanvasSize {
                width: 1080,
                height: 1080,
            },
            Self::Tall => CanvasSize {
                width: 1080,
                height: 1440,
            },
        })
    }
    /// The preset's pixel size, resolving `Original` from the retained source size.
    pub const fn resolved(self, original: Option<CanvasSize>) -> Option<CanvasSize> {
        match self {
            Self::Original => original,
            _ => self.size(),
        }
    }
    /// Stable automation and wire fragment.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Landscape => "16x9",
            Self::Standard => "4x3",
            Self::Wide => "2x1",
            Self::Portrait => "9x16",
            Self::Square => "1x1",
            Self::Tall => "3x4",
        }
    }
    /// The reference's ratio text; `Original` is localized by the caller.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Original => "Original",
            Self::Landscape => "16:9",
            Self::Standard => "4:3",
            Self::Wide => "2:1",
            Self::Portrait => "9:16",
            Self::Square => "1:1",
            Self::Tall => "3:4",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub fps: FrameRate,
    pub background: Color,
    /// Selected fixed preset; `None` after custom width/height entry.
    pub preset: Option<AspectPreset>,
    /// Last committed custom size, restored when the creator returns to custom.
    pub last_custom_size: CanvasSize,
}
const fn default_custom_size() -> CanvasSize {
    CanvasSize {
        width: 1920,
        height: 1080,
    }
}
impl Default for Canvas {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: FrameRate::FPS_30,
            background: Color([0, 0, 0, 255]),
            preset: Some(AspectPreset::Landscape),
            last_custom_size: default_custom_size(),
        }
    }
}
/// Output sample rate; presentation preferences stay in the app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub audio_sample_rate: u32,
    pub master_volume: f32,
    pub master_muted: bool,
    pub loudness: Loudness,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            audio_sample_rate: 48000,
            master_volume: 0.0,
            master_muted: false,
            loudness: Loudness::default(),
        }
    }
}
/// EBU R128 normalization target: integrated loudness and true-peak ceiling.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoudnessTarget {
    pub target_lufs: f32,
    pub true_peak_db: f32,
}
impl LoudnessTarget {
    /// Default target: -23 LUFS integrated (EBU R128 broadcast; the reference editor's default), -1 dBTP.
    pub const DEFAULT: Self = Self {
        target_lufs: -23.0,
        true_peak_db: -1.0,
    };
    /// Finite, on the hundredth grid: LUFS -50..=-5, true peak -20..=0 dBTP.
    pub fn is_valid(self) -> bool {
        let grid = |v: f32| (v * 100.0 - (v * 100.0).round()).abs() < 0.001;
        self.target_lufs.is_finite()
            && self.true_peak_db.is_finite()
            && (-50.0..=-5.0).contains(&self.target_lufs)
            && (-20.0..=0.0).contains(&self.true_peak_db)
            && grid(self.target_lufs)
            && grid(self.true_peak_db)
    }
}
/// Which part of the signal a loudness target normalizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LoudnessScope {
    Clip,
    Master,
}
/// Which loudness gains a project requests; `measured` holds the last two-pass result.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Loudness {
    /// Normalize each audible clip to this target before the master chain.
    pub clip: Option<LoudnessTarget>,
    /// Normalize the whole mix (after clip gains) to this target.
    pub master: Option<LoudnessTarget>,
    pub measured: Option<LoudnessGains>,
}
/// Gains derived by one measurement pass. `key` binds them to the audio content
/// and targets they were measured for; the mixer applies them unconditionally so
/// preview and export agree, and export admission refuses stale keys.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoudnessGains {
    pub key: [u8; 32],
    /// Per-clip dB gain, sorted by item; absent items (silence, identity) are unity.
    pub clip: Vec<(ItemId, f32)>,
    /// Gain in dB applied to the summed mix before the limiter.
    pub master_db: f32,
}
impl LoudnessGains {
    pub const MAX_CLIPS: usize = 4096;
    pub const GAIN_RANGE_DB: std::ops::RangeInclusive<f32> = -60.0..=60.0;
    pub fn is_valid(&self) -> bool {
        self.clip.len() <= Self::MAX_CLIPS
            && Self::GAIN_RANGE_DB.contains(&self.master_db)
            && self.master_db.is_finite()
            && self
                .clip
                .iter()
                .all(|(_, g)| g.is_finite() && Self::GAIN_RANGE_DB.contains(g))
            && self.clip.windows(2).all(|w| match w {
                [a, b] => a.0.0 < b.0.0,
                _ => true,
            })
    }
    pub fn clip_db(&self, item: ItemId) -> f32 {
        self.clip
            .binary_search_by_key(&item.0, |(id, _)| id.0)
            .map_or(0.0, |i| self.clip.get(i).map_or(0.0, |(_, g)| *g))
    }
}
