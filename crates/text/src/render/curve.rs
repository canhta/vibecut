//! Places glyphs on a circular arc, one arc per laid-out line.

use crate::layout::Layout;
use crate::raster::Path;

/// Arc angle a line spans at a curve of 100 percent, in radians (a half circle). Open: the
/// reference gives no measure.
const FULL_ARC: f32 = std::f32::consts::PI;

/// Largest curve magnitude, in percent.
pub const MAX_CURVE: f32 = 100.0;

/// Rigid placement of one glyph: its pivot (advance centre on the baseline) in layout
/// coordinates and the clockwise turn about it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Placement {
    /// Glyph-local x of the pivot.
    pub half: f32,
    pub x: f32,
    pub y: f32,
    pub sin: f32,
    pub cos: f32,
}

/// Placements for every glyph, or `None` at zero curve. A positive curve raises the middle of
/// each line over its ends (an arch) and a negative one lowers it; the line keeps its length
/// along the arc, centred on its own middle.
pub(super) fn placements(layout: &Layout, percent: f32) -> Option<Vec<Placement>> {
    let angle = percent / MAX_CURVE * FULL_ARC;
    if angle == 0.0 {
        return None;
    }
    let mut out: Vec<Placement> = layout
        .glyphs
        .iter()
        .map(|glyph| Placement {
            half: 0.0,
            x: glyph.x,
            y: glyph.y,
            sin: 0.0,
            cos: 1.0,
        })
        .collect();
    for line in &layout.lines {
        if !(line.width.is_finite() && line.width > 0.0) {
            continue;
        }
        let curvature = angle / line.width;
        let mid = line.x + line.width / 2.0;
        let range = line.glyphs.clone();
        let mut advance = layout
            .glyphs
            .get(range.start)
            .map_or(0.0, |glyph| glyph.size * 0.5);
        for index in range.clone() {
            let Some(glyph) = layout.glyphs.get(index) else {
                continue;
            };
            if let Some(next) = layout
                .glyphs
                .get(index + 1..range.end)
                .and_then(|rest| rest.iter().find(|n| n.x > glyph.x + 1e-3))
            {
                advance = next.x - glyph.x;
            }
            let half = advance / 2.0;
            let phi = (glyph.x + half - mid) * curvature;
            let (sin, cos) = phi.sin_cos();
            if let Some(slot) = out.get_mut(index) {
                *slot = Placement {
                    half,
                    x: mid + sin / curvature,
                    y: glyph.y + (1.0 - cos) / curvature,
                    sin,
                    cos,
                };
            }
        }
    }
    Some(out)
}

/// Move a glyph path (glyph-local, y down) to its placed position plus `origin`.
pub(super) fn place(path: &mut Path, at: &Placement, origin: (f32, f32)) {
    for contour in &mut path.contours {
        for point in contour.iter_mut() {
            let (rx, ry) = (point.0 - at.half, point.1);
            *point = (
                origin.0 + at.x + rx * at.cos - ry * at.sin,
                origin.1 + at.y + rx * at.sin + ry * at.cos,
            );
        }
    }
}

/// Pixels a line of `width` pixels moves off its straight position at `percent`: the arc's
/// sagitta, an upper bound on the vertical shift of any glyph pivot.
pub fn displacement(percent: f32, width: f32) -> f32 {
    let angle = (percent / MAX_CURVE * FULL_ARC).abs();
    if !(angle > 0.0 && width.is_finite() && width > 0.0) {
        return 0.0;
    }
    width * (1.0 - (angle / 2.0).cos()) / angle
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{LayoutOptions, layout};

    fn laid_out() -> Layout {
        layout(
            "Curve text",
            &LayoutOptions {
                size: 40.0,
                max_width: 600.0,
                ..LayoutOptions::default()
            },
        )
        .expect("layout")
    }

    #[test]
    fn zero_curve_is_identity() {
        assert!(placements(&laid_out(), 0.0).is_none());
        assert_eq!(displacement(0.0, 500.0), 0.0);
    }

    #[test]
    fn positive_arches_and_negative_dips_symmetrically() {
        let layout = laid_out();
        let up = placements(&layout, 60.0).expect("arc");
        let down = placements(&layout, -60.0).expect("arc");
        let (first, last) = (0, layout.glyphs.len() - 1);
        let base = layout.glyphs[first].y;
        // Ends sit below the middle for an arch (y down) and above it for a dip.
        assert!(up[first].y > base && up[last].y > base);
        assert!(down[first].y < base && down[last].y < base);
        // Left and right ends turn opposite ways.
        assert!(up[first].sin < 0.0 && up[last].sin > 0.0);
        assert!(down[first].sin > 0.0 && down[last].sin < 0.0);
        // The dip reflects the arch about the baseline.
        for (a, b) in up.iter().zip(&down) {
            assert!(((a.y - base) + (b.y - base)).abs() < 1e-3);
            assert!((a.x - b.x).abs() < 1e-3);
        }
    }

    #[test]
    fn pivots_lie_on_one_circle_and_stay_within_the_sagitta() {
        let layout = laid_out();
        let line = &layout.lines[0];
        let radius = line.width / FULL_ARC;
        let (cx, cy) = (line.x + line.width / 2.0, layout.glyphs[0].y + radius);
        let bound = displacement(100.0, line.width);
        for p in placements(&layout, 100.0).expect("arc") {
            let r = ((p.x - cx).powi(2) + (p.y - cy).powi(2)).sqrt();
            assert!((r - radius).abs() < 0.01 * radius, "{r} vs {radius}");
            assert!(p.y - layout.glyphs[0].y <= bound + 1e-3);
        }
    }
}
