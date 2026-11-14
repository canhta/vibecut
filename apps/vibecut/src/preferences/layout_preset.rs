//! The five named editor layouts.

use super::aggregate::DEFAULT_PANELS;
use serde::{Deserialize, Serialize};

/// Which panel sits where. The arrangements and column fractions are measured from the
/// reference at 1874 x 1142 (capture 2026-11-14f, logical px): columns 1842 px of room,
/// the first column starts at x 9. Choosing a preset also sets `panels`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutPreset {
    /// Media, player and properties side by side over the full-width timeline.
    #[default]
    Default,
    /// Default with a wide properties column (540 px).
    Media,
    /// Media and player over the properties panel and timeline; one seam at x 713.
    Generative,
    /// Properties as a full-height right column; the timeline stops under the player.
    Attributes,
    /// A full-height tall player at the right, properties beside the media.
    Vertical,
}
impl LayoutPreset {
    pub const ALL: [Self; 5] = [
        Self::Default,
        Self::Media,
        Self::Generative,
        Self::Attributes,
        Self::Vertical,
    ];
    /// The stable id used in command ids (`layout.preset.<id>`).
    pub const fn id(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Media => "media",
            Self::Generative => "generative",
            Self::Attributes => "attributes",
            Self::Vertical => "vertical",
        }
    }
    /// The preset's column and row fractions `[first, last, timeline]`; the middle
    /// column takes the rest. Generative has no last column: its value is the Default's.
    pub const fn panels(self) -> [f32; 3] {
        let [_, last, timeline] = DEFAULT_PANELS;
        match self {
            Self::Default => DEFAULT_PANELS,
            Self::Media => [403.0 / 1842.0, 540.0 / 1842.0, timeline],
            Self::Generative => [704.0 / 1842.0, last, timeline],
            Self::Attributes => [702.0 / 1842.0, 373.0 / 1842.0, timeline],
            Self::Vertical => [1068.0 / 1842.0, 403.0 / 1842.0, timeline],
        }
    }
}
