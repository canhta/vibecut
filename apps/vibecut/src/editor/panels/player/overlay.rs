//! Preview guide model: grid bounds, safe areas, overlay visibility, paint planes.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};

/// Fewest rows or columns a grid guide may draw.
pub const GRID_MIN: u8 = 1;
/// Most rows or columns a grid guide may draw.
pub const GRID_MAX: u8 = 24;

/// Left, top, right and bottom safe-area edges as fractions of the frame.
///
/// UNVERIFIED placeholder: no published TikTok value was checked. Measure it
/// against the platform before relying on it.
pub const TIKTOK_SAFE_AREA: [f32; 4] = [0.05, 0.10, 0.95, 0.80];

/// The guide the picker selects. Only the app paints it, so export never sees it.
///
/// Reels, Shorts and Spotlight have no frame geometry in the reference source,
/// so they draw nothing; their safe areas are open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Guide {
    Grid { rows: u8, cols: u8 },
    Tiktok,
    Reels,
    Shorts,
    Spotlight,
}

impl Default for Guide {
    fn default() -> Self {
        Self::Grid { rows: 3, cols: 3 }
    }
}

impl Guide {
    /// Whether `other` is the same picker choice, ignoring grid size.
    pub fn same_choice(self, other: Self) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }

    pub fn validate(self) -> Result<()> {
        match self {
            Self::Grid { rows, cols }
                if !(GRID_MIN..=GRID_MAX).contains(&rows)
                    || !(GRID_MIN..=GRID_MAX).contains(&cols) =>
            {
                Err(Error::Invalid("guide grid"))
            }
            _ => Ok(()),
        }
    }

    /// Interior boundaries as fractions of the frame, `(vertical, horizontal)`.
    /// Empty for every guide except the grid.
    pub fn grid_lines(self) -> (Vec<f32>, Vec<f32>) {
        match self {
            Self::Grid { rows, cols } => (boundaries(cols), boundaries(rows)),
            _ => (Vec::new(), Vec::new()),
        }
    }

    /// The safe-area rectangle as `[left, top, right, bottom]` fractions of the
    /// frame, or `None` when the guide draws no frame geometry.
    pub fn safe_area(self) -> Option<[f32; 4]> {
        match self {
            Self::Tiktok => Some(TIKTOK_SAFE_AREA),
            _ => None,
        }
    }
}

/// `count` equal cells need `count - 1` interior boundaries.
fn boundaries(count: u8) -> Vec<f32> {
    (1..count)
        .map(|i| f32::from(i) / f32::from(count))
        .collect()
}

/// Paint planes, back to front. A later plane draws above an earlier one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Plane {
    /// The composed frame image, painted before any overlay.
    Scene,
    /// Fixed over the displayed frame: the guide.
    Viewport,
    /// Screen-anchored notes drawn above the frame.
    Hud,
}

/// Every plane in paint order.
pub const PLANES: [Plane; 3] = [Plane::Scene, Plane::Viewport, Plane::Hud];

/// A preview overlay the player paints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    Notes,
    Guide,
}

impl Overlay {
    pub const fn plane(self) -> Plane {
        match self {
            Self::Notes => Plane::Hud,
            Self::Guide => Plane::Viewport,
        }
    }
}

/// Per-overlay visibility, persisted in the preference document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayVisibility {
    pub guide: bool,
    pub notes: bool,
}

impl Default for OverlayVisibility {
    fn default() -> Self {
        Self {
            guide: false,
            notes: true,
        }
    }
}

impl OverlayVisibility {
    pub const fn shows(self, overlay: Overlay) -> bool {
        match overlay {
            Overlay::Notes => self.notes,
            Overlay::Guide => self.guide,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn grid_rows_and_columns_accept_one_through_twenty_four() {
        for n in [GRID_MIN, 3, GRID_MAX] {
            assert!(Guide::Grid { rows: n, cols: n }.validate().is_ok());
        }
        for n in [0, GRID_MAX + 1, u8::MAX] {
            assert!(Guide::Grid { rows: n, cols: 3 }.validate().is_err());
            assert!(Guide::Grid { rows: 3, cols: n }.validate().is_err());
        }
    }

    #[test]
    fn grid_lines_are_one_fewer_than_the_cells_and_span_the_frame() {
        let (vertical, horizontal) = Guide::Grid { rows: 3, cols: 4 }.grid_lines();
        assert_eq!(vertical, vec![0.25, 0.5, 0.75]);
        assert_eq!(horizontal, vec![1.0 / 3.0, 2.0 / 3.0]);
        assert_eq!(
            Guide::Grid { rows: 1, cols: 1 }.grid_lines(),
            (Vec::new(), Vec::new())
        );
        assert_eq!(
            Guide::Grid {
                rows: GRID_MAX,
                cols: GRID_MAX
            }
            .grid_lines()
            .0
            .len(),
            usize::from(GRID_MAX) - 1
        );
    }

    #[test]
    fn planes_paint_scene_then_viewport_then_hud_and_notes_sit_above_the_guide() {
        assert_eq!(PLANES, [Plane::Scene, Plane::Viewport, Plane::Hud]);
        assert!(PLANES.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(Overlay::Guide.plane(), Plane::Viewport);
        assert_eq!(Overlay::Notes.plane(), Plane::Hud);
    }

    #[test]
    fn defaults_are_a_three_by_three_grid_with_notes_shown_and_the_guide_hidden() {
        assert_eq!(Guide::default(), Guide::Grid { rows: 3, cols: 3 });
        let visibility = OverlayVisibility::default();
        assert!(visibility.shows(Overlay::Notes));
        assert!(!visibility.shows(Overlay::Guide));
    }

    #[test]
    fn only_tiktok_has_frame_geometry_and_the_picker_ignores_grid_size() {
        assert!(Guide::Tiktok.safe_area().is_some());
        assert!(Guide::Reels.safe_area().is_none());
        assert!(Guide::Shorts.safe_area().is_none());
        assert!(Guide::Spotlight.safe_area().is_none());
        assert!(Guide::Grid { rows: 5, cols: 2 }.same_choice(Guide::default()));
        assert!(!Guide::Grid { rows: 5, cols: 2 }.same_choice(Guide::Tiktok));
    }
}
