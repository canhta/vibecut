//! Preview guide model: grid bounds, overlay visibility, paint planes.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};

/// Fewest rows or columns a grid guide may draw.
pub const GRID_MIN: u8 = 1;
/// Most rows or columns a grid guide may draw.
pub const GRID_MAX: u8 = 24;

/// The guide the picker selects. Only the app paints it, so export never sees it.
/// The platform variants shade the regions that platform's own UI covers; their
/// fractions and sources are listed in the editor UI doc.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Guide {
    None,
    Grid {
        rows: u8,
        cols: u8,
    },
    #[serde(rename = "tiktok")]
    TikTok,
    InstagramReels,
    #[serde(rename = "youtube_shorts")]
    YouTubeShorts,
}

/// One UI-covered region as fractions of the frame: `[left, top, right, bottom]`.
pub type Region = [f32; 4];

/// The guide choices the picker lists after None and Grid, one row per platform.
pub const PLATFORM_GUIDES: [Guide; 3] =
    [Guide::TikTok, Guide::InstagramReels, Guide::YouTubeShorts];

/// Covered bands of a 9:16 frame: full-width top and bottom bands and the left
/// and right columns between them.
fn banded(top: f32, bottom: f32, left: f32, right: f32) -> Vec<Region> {
    vec![
        [0.0, 0.0, 1.0, top],
        [0.0, 1.0 - bottom, 1.0, 1.0],
        [0.0, top, left, 1.0 - bottom],
        [1.0 - right, top, 1.0, 1.0 - bottom],
    ]
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
    /// Empty when no guide is picked.
    pub fn grid_lines(self) -> (Vec<f32>, Vec<f32>) {
        match self {
            Self::Grid { rows, cols } => (boundaries(cols), boundaries(rows)),
            Self::None | Self::TikTok | Self::InstagramReels | Self::YouTubeShorts => {
                (Vec::new(), Vec::new())
            }
        }
    }

    /// The regions the platform's UI covers, disjoint, as fractions of the frame.
    /// Empty for None and Grid.
    pub fn covered_regions(self) -> Vec<Region> {
        match self {
            // TikTok In-Feed safe-zone file, 720x1280: 80 px sides, 160 px top, 440 px
            // bottom, plus a 120 px wide rail on the right reaching 720 px up from the
            // bottom, inside the safe area.
            Self::TikTok => {
                let mut regions =
                    banded(160.0 / 1280.0, 440.0 / 1280.0, 80.0 / 720.0, 80.0 / 720.0);
                regions.push([
                    520.0 / 720.0,
                    560.0 / 1280.0,
                    640.0 / 720.0,
                    1.0 - 440.0 / 1280.0,
                ]);
                regions
            }
            // Meta Reels ad guidance: 14% top, 35% bottom, 6% on each side.
            Self::InstagramReels => banded(0.14, 0.35, 0.06, 0.06),
            // Google vertical video template, 1080x1920: safe rectangle x 48..888,
            // y 288..1248.
            Self::YouTubeShorts => banded(
                288.0 / 1920.0,
                672.0 / 1920.0,
                48.0 / 1080.0,
                192.0 / 1080.0,
            ),
            Self::None | Self::Grid { .. } => Vec::new(),
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
    fn the_picker_ignores_grid_size_and_no_guide_draws_no_lines() {
        assert!(Guide::Grid { rows: 5, cols: 2 }.same_choice(Guide::default()));
        assert!(!Guide::Grid { rows: 5, cols: 2 }.same_choice(Guide::None));
        assert_eq!(Guide::None.grid_lines(), (Vec::new(), Vec::new()));
        assert!(Guide::TikTok.same_choice(Guide::TikTok));
        assert!(!Guide::TikTok.same_choice(Guide::InstagramReels));
        for guide in PLATFORM_GUIDES {
            assert_eq!(guide.grid_lines(), (Vec::new(), Vec::new()));
            assert!(guide.validate().is_ok());
        }
    }

    fn area(regions: &[Region]) -> f32 {
        regions.iter().map(|r| (r[2] - r[0]) * (r[3] - r[1])).sum()
    }

    #[test]
    fn platform_regions_match_the_measured_margins_and_never_overlap() {
        assert!(Guide::None.covered_regions().is_empty());
        assert!(Guide::default().covered_regions().is_empty());
        let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
        // Safe area of each platform: the frame minus the covered area.
        let tiktok = Guide::TikTok.covered_regions();
        assert!(near(tiktok[0][3], 0.125), "top 160/1280");
        assert!(near(tiktok[1][1], 0.65625), "bottom 440/1280");
        assert!(near(tiktok[2][2], 1.0 / 9.0), "side 80/720");
        assert!(near(tiktok[4][0], 520.0 / 720.0) && near(tiktok[4][1], 0.4375));
        let reels = Guide::InstagramReels.covered_regions();
        assert!(near(reels[0][3], 0.14) && near(reels[1][1], 0.65) && near(reels[2][2], 0.06));
        let shorts = Guide::YouTubeShorts.covered_regions();
        assert!(near(shorts[0][3], 0.15) && near(shorts[1][1], 0.65));
        assert!(near(shorts[2][2], 48.0 / 1080.0) && near(shorts[3][0], 0.8));
        for guide in PLATFORM_GUIDES {
            let regions = guide.covered_regions();
            for (i, a) in regions.iter().enumerate() {
                assert!(a[0] < a[2] && a[1] < a[3], "{guide:?} region is non-empty");
                for b in &regions[i + 1..] {
                    let eps = 1e-4;
                    let overlap = a[0] + eps < b[2]
                        && b[0] + eps < a[2]
                        && a[1] + eps < b[3]
                        && b[1] + eps < a[3];
                    assert!(!overlap, "{guide:?} regions are disjoint");
                }
            }
            let covered = area(&regions);
            assert!(covered > 0.3 && covered < 0.7, "{guide:?} covers {covered}");
        }
    }

    #[test]
    fn platform_guides_serialize_under_stable_kinds_and_refuse_unknown_ones() {
        for (guide, kind) in [
            (Guide::TikTok, "tiktok"),
            (Guide::InstagramReels, "instagram_reels"),
            (Guide::YouTubeShorts, "youtube_shorts"),
        ] {
            let value = serde_json::json!(guide);
            assert_eq!(value, serde_json::json!({ "kind": kind }));
            assert_eq!(serde_json::from_value::<Guide>(value).unwrap(), guide);
        }
        assert!(serde_json::from_value::<Guide>(serde_json::json!({"kind": "spotlight"})).is_err());
    }
}
