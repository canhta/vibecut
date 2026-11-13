//! Resolves canvas snap targets and guides for preview transform gestures.
//!
//! The candidate set is spatial: the canvas centre and edges plus the visible
//! box edges of every other item at the playhead. Ranking, the screen-pixel
//! threshold and the disabled/bypass identity follow the timeline's rule: the
//! shared [`closest`] rank with this module's own measured thresholds.

use crate::editor::{
    context::closest,
    panels::player::state::{PreviewAxis, PreviewGuide},
};
use project::ItemId;
use render::{
    Rect,
    viewport::{ItemGeometry, Viewport},
};

/// Preview transform threshold in logical screen pixels, measured in the
/// reference editor on 2026-11-10 at its 100 % canvas zoom: an item 5 px from a
/// guide still snapped and 6 px did not.
pub const PREVIEW_SNAP_PIXELS: f64 = 5.0;

/// Preview rotation snaps to quarter turns within this many degrees. The
/// implementation reference measures the same 90/5 pair
/// (`apps/web/src/preview/preview-snap.ts`: `ROTATE_SNAP_STEP_DEGREES`,
/// `ROTATE_SNAP_THRESHOLD_DEGREES`).
const ROTATE_SNAP_STEP: f64 = 90.0;
const ROTATE_SNAP_DEGREES: f64 = 5.0;

/// Stable identity for a canvas snap target; the variant order breaks equal
/// distances so a tie cannot change between frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Source {
    CanvasLeft,
    CanvasCentreX,
    CanvasRight,
    CanvasTop,
    CanvasCentreY,
    CanvasBottom,
    Item(ItemId, Edge),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

/// Canvas lines a gesture can snap to, in project canvas pixels.
pub(in crate::editor) struct Targets {
    /// x positions of vertical lines.
    vertical: Vec<(f64, Source)>,
    /// y positions of horizontal lines.
    horizontal: Vec<(f64, Source)>,
}

impl Targets {
    /// Canvas centre and edges, plus every other item's transformed box edges.
    pub(in crate::editor) fn new(
        canvas: [u32; 2],
        items: impl IntoIterator<Item = (ItemId, Rect)>,
    ) -> Self {
        let [width, height] = canvas.map(f64::from);
        let mut targets = Self {
            vertical: vec![
                (0.0, Source::CanvasLeft),
                (width / 2.0, Source::CanvasCentreX),
                (width, Source::CanvasRight),
            ],
            horizontal: vec![
                (0.0, Source::CanvasTop),
                (height / 2.0, Source::CanvasCentreY),
                (height, Source::CanvasBottom),
            ],
        };
        for (item, extent) in items {
            targets
                .vertical
                .push((extent.x, Source::Item(item, Edge::Left)));
            targets
                .vertical
                .push((extent.x + extent.width, Source::Item(item, Edge::Right)));
            targets
                .horizontal
                .push((extent.y, Source::Item(item, Edge::Top)));
            targets
                .horizontal
                .push((extent.y + extent.height, Source::Item(item, Edge::Bottom)));
        }
        targets
    }

    /// Snap a proposed move on both axes independently: the box's centre and
    /// both edges may each meet a target line.
    pub(in crate::editor) fn move_by(
        &self,
        extent: Rect,
        threshold: f64,
    ) -> ([f64; 2], Vec<PreviewGuide>) {
        let mut delta = [0.0, 0.0];
        let mut guides = Vec::new();
        let x_edges = [
            extent.x,
            extent.x + extent.width / 2.0,
            extent.x + extent.width,
        ];
        let y_edges = [
            extent.y,
            extent.y + extent.height / 2.0,
            extent.y + extent.height,
        ];
        if let Some((adjust, at)) = snap_axis(&x_edges, &self.vertical, threshold) {
            delta[0] = adjust;
            guides.push(PreviewGuide {
                axis: PreviewAxis::Vertical,
                position: at,
            });
        }
        if let Some((adjust, at)) = snap_axis(&y_edges, &self.horizontal, threshold) {
            delta[1] = adjust;
            guides.push(PreviewGuide {
                axis: PreviewAxis::Horizontal,
                position: at,
            });
        }
        (delta, guides)
    }

    /// Snap the dragged handle point on each axis the handle moves, so corner
    /// handles snap both axes and side handles only their own.
    pub(in crate::editor) fn handle(
        &self,
        point: [f64; 2],
        across: bool,
        down: bool,
        threshold: f64,
    ) -> ([f64; 2], Vec<PreviewGuide>) {
        let mut point = point;
        let mut guides = Vec::new();
        if across && let Some((adjust, at)) = snap_axis(&[point[0]], &self.vertical, threshold) {
            point[0] += adjust;
            guides.push(PreviewGuide {
                axis: PreviewAxis::Vertical,
                position: at,
            });
        }
        if down && let Some((adjust, at)) = snap_axis(&[point[1]], &self.horizontal, threshold) {
            point[1] += adjust;
            guides.push(PreviewGuide {
                axis: PreviewAxis::Horizontal,
                position: at,
            });
        }
        (point, guides)
    }
}

/// The lines a gesture on `item` can snap to at `at`: the canvas centre and edges and
/// every other visible item's transformed box edges. Transform and mask gestures share it.
pub(in crate::editor) fn targets(
    project: &project::Project,
    renderer: &render::Renderer<'_>,
    item: ItemId,
    at: time::Tick,
) -> Targets {
    Targets::new(
        [project.data().canvas.width, project.data().canvas.height],
        project
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter())
            .filter(|candidate| candidate.id != item)
            .filter_map(|candidate| {
                let geometry = renderer.item_geometry(candidate.id, at).ok().flatten()?;
                Some((candidate.id, extent(&geometry)))
            }),
    )
}

/// The preview threshold in canvas pixels at the current viewport scale.
/// Hostile geometry cannot form a threshold and allows only an exact match.
pub(in crate::editor) fn threshold(viewport: &Viewport) -> f64 {
    viewport.canvas_distance(PREVIEW_SNAP_PIXELS).unwrap_or(0.0)
}

/// Quarter-turn snapping: the nearest step when the raw angle is within the
/// shared degree threshold.
pub(in crate::editor) fn rotation(degrees: f64) -> Option<f64> {
    if !degrees.is_finite() {
        return None;
    }
    let snapped = (degrees / ROTATE_SNAP_STEP).round() * ROTATE_SNAP_STEP;
    ((degrees - snapped).abs() <= ROTATE_SNAP_DEGREES).then_some(snapped)
}

/// The guide a snapped rotation shows: the item's own axis at the quarter turn.
pub(in crate::editor) fn rotation_guide(degrees: f64, centre: [f64; 2]) -> PreviewGuide {
    if degrees.rem_euclid(180.0) == 0.0 {
        PreviewGuide {
            axis: PreviewAxis::Horizontal,
            position: centre[1],
        }
    } else {
        PreviewGuide {
            axis: PreviewAxis::Vertical,
            position: centre[0],
        }
    }
}

/// Axis-aligned box of a transformed item in project canvas pixels. The
/// envelope math is render-owned; the player only consumes it.
pub(in crate::editor) fn extent(geometry: &ItemGeometry) -> Rect {
    geometry.transformed_bounds()
}

/// The closest target for any of `edges`, as `(adjustment, target)`.
fn snap_axis(edges: &[f64], targets: &[(f64, Source)], threshold: f64) -> Option<(f64, f64)> {
    closest(edges.iter().flat_map(|&edge| {
        targets.iter().filter_map(move |&(at, source)| {
            let adjust = at - edge;
            (adjust.abs() <= threshold).then_some(((adjust, at), source, adjust.abs()))
        })
    }))
    .map(|((adjust, at), _, _)| (adjust, at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use project::{
        AnimationPath, ItemParams, Param, ParamValue, Project, ProjectId, TextParams, Track,
        TrackId, TrackKind, TransformField,
    };
    use time::{SourceRange, TICKS_PER_SECOND, Tick};

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    fn viewport(zoom: f64, pan: [f64; 2]) -> Viewport {
        Viewport::new(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 640.0,
                height: 360.0,
            },
            [640, 360],
            zoom,
            pan,
        )
        .unwrap()
    }

    fn canvas() -> Targets {
        Targets::new([640, 360], [])
    }

    #[test]
    fn targets_carry_canvas_centre_edges_and_other_item_extents() {
        let targets = Targets::new([640, 360], [(ItemId(2), rect(10.0, 20.0, 30.0, 40.0))]);
        let vertical: Vec<f64> = targets.vertical.iter().map(|(at, _)| *at).collect();
        let horizontal: Vec<f64> = targets.horizontal.iter().map(|(at, _)| *at).collect();
        assert_eq!(vertical, vec![0.0, 320.0, 640.0, 10.0, 40.0]);
        assert_eq!(horizontal, vec![0.0, 180.0, 360.0, 20.0, 60.0]);
    }

    #[test]
    fn move_snaps_each_axis_independently_and_reports_guides() {
        let targets = canvas();
        // The right edge is 3 px short of the canvas right edge.
        let (delta, guides) = targets.move_by(rect(537.0, 100.0, 100.0, 100.0), 5.0);
        assert_eq!(delta, [3.0, 0.0]);
        assert_eq!(
            guides,
            vec![PreviewGuide {
                axis: PreviewAxis::Vertical,
                position: 640.0,
            }]
        );
        // Both axes meet the canvas centre with no adjustment at all.
        let (delta, guides) = targets.move_by(rect(300.0, 170.0, 40.0, 20.0), 5.0);
        assert_eq!(delta, [0.0, 0.0]);
        assert_eq!(guides.len(), 2);
        // Nothing within the threshold keeps the proposed box and shows no guide.
        let (delta, guides) = targets.move_by(rect(120.0, 60.0, 40.0, 20.0), 5.0);
        assert_eq!(delta, [0.0, 0.0]);
        assert!(guides.is_empty());
    }

    #[test]
    fn move_prefers_the_closest_edge_and_ignores_far_targets() {
        let targets = Targets::new([640, 360], [(ItemId(2), rect(200.0, 100.0, 40.0, 40.0))]);
        // Left edge 203 is 3 px from the other item's left edge and the box's
        // centre 243 is 3 px from the other item's right edge; the stable source
        // order decides, and the closer of the two wins.
        let (delta, guides) = targets.move_by(rect(203.0, 300.0, 80.0, 40.0), 5.0);
        assert_eq!(delta, [-3.0, 0.0], "the closest edge moves to its target");
        assert_eq!(guides[0].position, 200.0);
        // A target outside the threshold never moves the box.
        let (delta, guides) = targets.move_by(rect(190.0, 300.0, 80.0, 40.0), 5.0);
        assert_eq!(delta, [0.0, 0.0]);
        assert!(guides.is_empty());
    }

    #[test]
    fn scale_handle_snaps_only_the_axes_it_moves() {
        let targets = canvas();
        let (point, guides) = targets.handle([637.0, 500.0], true, false, 5.0);
        assert_eq!(point, [640.0, 500.0]);
        assert_eq!(
            guides,
            vec![PreviewGuide {
                axis: PreviewAxis::Vertical,
                position: 640.0,
            }]
        );
        let (point, guides) = targets.handle([637.0, 177.0], true, true, 5.0);
        assert_eq!(point, [640.0, 180.0]);
        assert_eq!(guides.len(), 2, "a corner snaps both axes");
        let (point, guides) = targets.handle([637.0, 177.0], false, true, 5.0);
        assert_eq!(point, [637.0, 180.0]);
        assert_eq!(guides.len(), 1);
    }

    #[test]
    fn rotation_snaps_within_five_degrees_of_a_quarter_turn() {
        for (raw, expected) in [
            (0.0, Some(0.0)),
            (5.0, Some(0.0)),
            (5.1, None),
            (-5.0, Some(0.0)),
            (87.0, Some(90.0)),
            (93.0, Some(90.0)),
            (95.1, None),
            (180.0, Some(180.0)),
            (-93.0, Some(-90.0)),
            (267.0, Some(270.0)),
            (f64::NAN, None),
        ] {
            assert_eq!(rotation(raw), expected, "rotation {raw}");
        }
    }

    #[test]
    fn rotation_guide_follows_the_quarter_turn_axis() {
        assert_eq!(
            rotation_guide(90.0, [10.0, 20.0]),
            PreviewGuide {
                axis: PreviewAxis::Vertical,
                position: 10.0,
            }
        );
        assert_eq!(
            rotation_guide(180.0, [10.0, 20.0]),
            PreviewGuide {
                axis: PreviewAxis::Horizontal,
                position: 20.0,
            }
        );
        assert_eq!(
            rotation_guide(-90.0, [10.0, 20.0]),
            PreviewGuide {
                axis: PreviewAxis::Vertical,
                position: 10.0,
            }
        );
    }

    #[test]
    fn threshold_converts_the_shared_pixels_at_each_zoom_and_pan() {
        for (zoom, pan, expected) in [
            (1.0, [0.0, 0.0], 5.0),
            (2.0, [0.0, 0.0], 2.5),
            (0.5, [0.0, 0.0], 10.0),
            (2.0, [120.0, -40.0], 2.5),
        ] {
            let viewport = viewport(zoom, pan);
            assert_eq!(threshold(&viewport), expected, "zoom {zoom} pan {pan:?}");
        }
        // At 2x, 2.4 canvas px still snaps and 2.6 canvas px does not.
        let viewport = viewport(2.0, [0.0, 0.0]);
        let threshold = threshold(&viewport);
        let (delta, guides) = canvas().move_by(rect(635.2, 100.0, 2.4, 20.0), threshold);
        assert!(
            (delta[0] - 2.4).abs() < 1e-9,
            "2.4 canvas px is inside the 2.5 px threshold"
        );
        assert_eq!(guides.len(), 1);
        let (delta, guides) = canvas().move_by(rect(634.8, 100.0, 2.6, 20.0), threshold);
        assert_eq!(delta[0], 0.0);
        assert!(guides.is_empty(), "2.6 canvas px is outside the threshold");
    }

    #[test]
    fn extent_is_the_transformed_box_in_canvas_pixels() {
        let mut data = Project::empty(ProjectId(23), "Snap".into())
            .unwrap()
            .into_data();
        data.canvas.width = 640;
        data.canvas.height = 360;
        let mut item = testkit::item::item(ItemId(1), ItemParams::Text(TextParams::new("Snap")))
            .source(SourceRange::new(Tick::ZERO, Tick(20 * TICKS_PER_SECOND)).unwrap())
            .build();
        item.animations
            .set(
                AnimationPath::from(TransformField::Rotate.builtin()),
                Param::new(ParamValue::Scalar(90.0)).unwrap(),
            )
            .unwrap();
        data.scene_mut().timeline.insert_track(Track {
            id: TrackId(2),
            name: "Text".into(),
            kind: TrackKind::Text,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![item],
        });
        let project = Project::new(data).unwrap();
        let renderer =
            render::Renderer::new(&project, render::RenderSettings::from_project(&project))
                .unwrap();
        let geometry = renderer
            .item_geometry(ItemId(1), Tick::ZERO)
            .unwrap()
            .unwrap();
        let extent = extent(&geometry);
        let bounds = geometry.bounds();
        assert!(
            (extent.width - bounds.height).abs() < 1e-6,
            "a quarter turn swaps the box axes: {} vs {}",
            extent.width,
            bounds.height
        );
        assert!((extent.height - bounds.width).abs() < 1e-6);
        let centre = [
            extent.x + extent.width / 2.0,
            extent.y + extent.height / 2.0,
        ];
        assert!((centre[0] - (bounds.x + bounds.width / 2.0)).abs() < 1e-6);
        assert!((centre[1] - (bounds.y + bounds.height / 2.0)).abs() < 1e-6);
    }

    #[test]
    fn preview_thresholds_are_the_measured_pixel_and_degree_values() {
        assert_eq!(PREVIEW_SNAP_PIXELS, 5.0);
        assert_eq!(ROTATE_SNAP_STEP, 90.0);
        assert_eq!(ROTATE_SNAP_DEGREES, 5.0);
    }
}
