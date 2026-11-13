//! Whole-mask move, resize and rotate gestures on the preview canvas.
//!
//! The selected path mask's box (the unit square its path lives in) is outlined with four
//! corner handles and a rotation grip. A gesture writes the mask's own `center_x`/`center_y`,
//! `scale` or `rotation` as an absolute candidate from the press, staged as `masks.update`
//! inside `timeline.preview`; one drop is one undo step. Like the inspector it edits the
//! static fields, so a parameter driven by keyframes keeps its animated value. Resize is
//! uniform (the path's own `sizeMode`), so the corners scale about the mask centre.
//!
//! Positions are mapped through `render::viewport::MaskGeometry`, so the handles stay on the
//! rendered box under mask and item rotation and scale. Snapping uses the preview snap owner:
//! the moved box's edges and centre meet the canvas and other items' lines, the dragged
//! corner meets them per axis, and rotation meets quarter turns.

use super::{
    Action, Context, snap,
    state::PreviewGuide,
    transform::{paint_guides, paint_handle},
};
use crate::{
    egui::{self, CursorIcon, Pos2, Sense, Vec2},
    i18n::Text,
    kit,
};
use project::{ItemId, Mask};
use render::{
    Rect,
    viewport::{MaskGeometry, Viewport},
};
use time::Tick;

/// The mask `scale` range the model accepts.
const SCALE_RANGE: (f64, f64) = (0.001, 500.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Move,
    Resize,
    Rotate,
}

#[derive(Default)]
pub(super) struct State {
    drag: Option<Drag>,
}

impl State {
    /// Abandon a live gesture; the caller discards its staged candidate.
    pub(super) fn cancel(&mut self) -> bool {
        self.drag.take().is_some()
    }

    /// Start a gesture from a press at `press` (logical screen pixels). Returns whether
    /// one began: the mask must still exist and its geometry must be representable.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn begin(
        &mut self,
        project: &project::Project,
        renderer: &render::Renderer<'_>,
        viewport: &Viewport,
        item: ItemId,
        mask: &str,
        geometry: MaskGeometry,
        kind: Kind,
        press: [f64; 2],
        at: Tick,
    ) -> bool {
        let Some(start) = project
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter())
            .find(|candidate| candidate.id == item)
            .and_then(|candidate| candidate.masks.iter().find(|m| m.id == mask))
            .cloned()
        else {
            return false;
        };
        let Some(extent) = extent(&geometry, viewport) else {
            return false;
        };
        self.drag = Some(Drag {
            item,
            kind,
            start,
            geometry,
            targets: snap::targets(project, renderer, item, at),
            guides: Vec::new(),
            press,
            extent,
            last: None,
        });
        true
    }
}

struct Drag {
    item: ItemId,
    kind: Kind,
    /// The mask as stored at the press; every candidate derives from it.
    start: Mask,
    /// Mask geometry at the press, so the mapping stays fixed for the whole gesture.
    geometry: MaskGeometry,
    /// Canvas lines this gesture may snap to, captured at the press.
    targets: snap::Targets,
    guides: Vec<PreviewGuide>,
    /// Press position in logical screen pixels.
    press: [f64; 2],
    /// Axis-aligned box of the mask at the press, in visible canvas pixels.
    extent: Rect,
    last: Option<Mask>,
}

impl Drag {
    /// Absolute mask from the press for the pointer position. `enabled` is the shared
    /// snapping preference; `bypass` is the Shift rule the other preview gestures use.
    fn candidate(
        &mut self,
        viewport: &Viewport,
        pointer: [f64; 2],
        enabled: bool,
        bypass: bool,
    ) -> Option<Mask> {
        let active = enabled && !bypass;
        let threshold = snap::threshold(viewport);
        let frame = self.geometry.frame();
        if frame.width <= 0.0 || frame.height <= 0.0 {
            return None;
        }
        self.guides.clear();
        let mut mask = self.start.clone();
        match self.kind {
            Kind::Move => {
                let now = viewport.screen_to_canvas(pointer).ok()?;
                let from = viewport.screen_to_canvas(self.press).ok()?;
                let mut delta = [now[0] - from[0], now[1] - from[1]];
                if active {
                    let proposed = Rect {
                        x: self.extent.x + delta[0],
                        y: self.extent.y + delta[1],
                        ..self.extent
                    };
                    let (adjust, guides) = self.targets.move_by(proposed, threshold);
                    delta[0] += adjust[0];
                    delta[1] += adjust[1];
                    self.guides = guides;
                }
                // The visible centre at the press plus the snapped delta, mapped back
                // through the item transform into the placement frame.
                let centre = viewport
                    .screen_to_canvas(self.geometry.point(viewport, [0.5, 0.5]).ok()?)
                    .ok()?;
                let target = viewport
                    .canvas_to_screen([centre[0] + delta[0], centre[1] + delta[1]])
                    .ok()?;
                let to = self.geometry.frame_point(viewport, target).ok()?;
                mask.center_x = ((to[0] - frame.x) / frame.width - 0.5).clamp(-100.0, 100.0);
                mask.center_y = ((to[1] - frame.y) / frame.height - 0.5).clamp(-100.0, 100.0);
            }
            Kind::Resize => {
                let mut canvas = viewport.screen_to_canvas(pointer).ok()?;
                if active {
                    let (snapped, guides) = self.targets.handle(canvas, true, true, threshold);
                    canvas = snapped;
                    self.guides = guides;
                }
                let screen = viewport.canvas_to_screen(canvas).ok()?;
                let to = self.geometry.frame_point(viewport, screen).ok()?;
                let centre = self.geometry.centre();
                let [width, height] = self.geometry.size();
                // A corner sits half a diagonal from the centre whatever the rotation.
                let half = (width / 2.0).hypot(height / 2.0);
                if half <= 0.0 || !half.is_finite() {
                    return None;
                }
                let reach = (to[0] - centre[0]).hypot(to[1] - centre[1]);
                mask.scale = (self.start.scale * reach / half).clamp(SCALE_RANGE.0, SCALE_RANGE.1);
            }
            Kind::Rotate => {
                let centre = self.geometry.centre();
                let angle = |screen: [f64; 2]| -> Option<f64> {
                    let at = self.geometry.frame_point(viewport, screen).ok()?;
                    Some((at[1] - centre[1]).atan2(at[0] - centre[0]).to_degrees())
                };
                let delta = angle(pointer)? - angle(self.press)?;
                let delta = (delta + 180.0).rem_euclid(360.0) - 180.0;
                let mut rotation = self.start.rotation + delta;
                if active && let Some(snapped) = snap::rotation(rotation) {
                    rotation = snapped;
                    let visible = viewport
                        .screen_to_canvas(self.geometry.point(viewport, [0.5, 0.5]).ok()?)
                        .ok()?;
                    self.guides.push(snap::rotation_guide(snapped, visible));
                }
                mask.rotation = rotation.clamp(-1e6, 1e6);
            }
        }
        Some(mask)
    }

    /// A gesture that returned to its origin publishes nothing; the edited fields compare
    /// within float noise because every candidate goes through the viewport round trip.
    fn commit(&self) -> bool {
        self.last.as_ref().is_some_and(|mask| {
            [
                (mask.center_x, self.start.center_x),
                (mask.center_y, self.start.center_y),
                (mask.scale, self.start.scale),
                (mask.rotation, self.start.rotation),
            ]
            .iter()
            .any(|(now, before)| (now - before).abs() > 1e-9)
        })
    }
}

/// Axis-aligned box of the mask's four corners in visible canvas pixels.
fn extent(geometry: &MaskGeometry, viewport: &Viewport) -> Option<Rect> {
    let (mut left, mut top) = (f64::MAX, f64::MAX);
    let (mut right, mut bottom) = (f64::MIN, f64::MIN);
    for local in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
        let screen = geometry.point(viewport, local).ok()?;
        let [x, y] = viewport.screen_to_canvas(screen).ok()?;
        left = left.min(x);
        right = right.max(x);
        top = top.min(y);
        bottom = bottom.max(y);
    }
    Some(Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

/// Continue the gesture in progress. Returns `true` while it owns the pointer.
#[allow(clippy::too_many_arguments)]
pub(super) fn drive(
    state: &mut State,
    cx: &Context<'_>,
    ui: &egui::Ui,
    viewport: &Viewport,
    frame: egui::Rect,
    pointer: Option<Pos2>,
    down: bool,
    bypass: bool,
    actions: &mut Vec<Action>,
) -> bool {
    let Some(mut drag) = state.drag.take() else {
        return false;
    };
    if down {
        if let Some(pointer) = pointer
            && let Some(mask) = drag.candidate(
                viewport,
                [f64::from(pointer.x), f64::from(pointer.y)],
                cx.preferences.snapping,
                bypass,
            )
            && drag.last.as_ref() != Some(&mask)
        {
            drag.last = Some(mask.clone());
            actions.push(Action::MaskBoxPreview {
                item: drag.item,
                mask,
            });
        }
        let canvas = &cx.staged_project().data().canvas;
        paint_guides(
            ui,
            viewport,
            [canvas.width, canvas.height],
            &drag.guides,
            cx.dark,
            frame,
        );
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        state.drag = Some(drag);
    } else {
        // A press that never moved is a click: it still leaves the point scope.
        if drag.last.is_none() && drag.kind == Kind::Move && !cx.mask_points.is_empty() {
            actions.push(Action::MaskPointClear);
        }
        actions.push(Action::MaskPathEnd {
            commit: drag.commit(),
        });
    }
    true
}

/// Screen positions of the box handles.
pub(super) struct Handles {
    corners: [Pos2; 4],
    grip: Pos2,
    top: Pos2,
    centre: Pos2,
}

impl Handles {
    pub(super) fn new(geometry: &MaskGeometry, viewport: &Viewport) -> Option<Self> {
        let at = |local: [f64; 2]| {
            geometry
                .point(viewport, local)
                .ok()
                .map(|[x, y]| egui::pos2(x as f32, y as f32))
        };
        let top = at([0.5, 0.0])?;
        let centre = at([0.5, 0.5])?;
        let away = top - centre;
        let length = away.length();
        let grip = if length > 0.0 {
            top + away / length * kit::size::TRANSFORM_GAP
        } else {
            top
        };
        Some(Self {
            corners: [
                at([0.0, 0.0])?,
                at([1.0, 0.0])?,
                at([1.0, 1.0])?,
                at([0.0, 1.0])?,
            ],
            grip,
            top,
            centre,
        })
    }

    /// The handle under `point`: the rotation grip first, then a corner.
    pub(super) fn hit(&self, point: Pos2) -> Option<Kind> {
        if point.distance(self.grip) <= kit::size::TRANSFORM_HIT {
            return Some(Kind::Rotate);
        }
        self.corners
            .iter()
            .any(|corner| point.distance(*corner) <= kit::size::TRANSFORM_HIT)
            .then_some(Kind::Resize)
    }

    pub(super) fn paint(&self, ui: &egui::Ui, dark: bool) {
        let stroke = egui::Stroke::new(kit::size::TRANSFORM_STROKE, kit::accent(dark));
        let painter = ui.painter();
        let [a, b, c, d] = self.corners;
        for (from, to) in [(a, b), (b, c), (c, d), (d, a)] {
            painter.line_segment([from, to], stroke);
        }
        painter.line_segment([self.top, self.grip], stroke);
        for corner in self.corners {
            paint_handle(ui, corner, kit::size::TRANSFORM_HANDLE / 2.0, dark);
        }
        paint_handle(ui, self.grip, kit::size::TRANSFORM_GRIP / 2.0, dark);
    }

    /// Register one stable automation target for the move centre, each corner and the grip.
    pub(super) fn remember(&self, ui: &mut egui::Ui, cx: &Context<'_>, actions: &mut Vec<Action>) {
        let mut targets = vec![
            (
                "preview.mask.box.move".to_owned(),
                self.centre,
                Text::Position,
            ),
            (
                "preview.mask.box.rotate".to_owned(),
                self.grip,
                Text::Rotate,
            ),
        ];
        for (index, corner) in self.corners.iter().enumerate() {
            targets.push((
                format!("preview.mask.box.corner.{index}"),
                *corner,
                Text::Scale,
            ));
        }
        for (id, position, label) in targets {
            let response = ui.interact(
                egui::Rect::from_center_size(position, Vec2::splat(kit::size::TRANSFORM_HIT * 2.0)),
                ui.make_persistent_id(&id),
                Sense::click_and_drag(),
            );
            let label = cx.tr(label);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
            actions.push(Action::Remember {
                id,
                icon: None,
                tooltip: None,
                response,
            });
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::editor::panels::player::state::PreviewAxis;
    use project::{
        Color, ItemParams, MaskPath, MaskShape, OverlayParams, Project, ProjectId, Track, TrackId,
        TrackKind,
    };
    use time::{SourceRange, TICKS_PER_SECOND};

    /// A 200x100 canvas whose only item carries one 80x40 path mask centred 20 px right
    /// of the canvas centre; the viewport maps canvas pixels to screen pixels 1:1.
    fn gesture(kind: Kind, press: [f64; 2]) -> (Drag, Viewport) {
        let mut mask = Mask::new(
            "m",
            MaskShape::Path {
                path: MaskPath::ellipse(),
            },
        );
        mask.width = 0.4;
        mask.height = 0.4;
        mask.center_x = 0.1;
        let mut data = Project::empty(ProjectId(37), "Mask".into())
            .unwrap()
            .into_data();
        data.canvas.width = 200;
        data.canvas.height = 100;
        data.scene_mut().timeline.insert_track(Track {
            id: TrackId(1),
            name: "V".into(),
            kind: TrackKind::Video,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![
                testkit::item::item(
                    ItemId(1),
                    ItemParams::Overlay(OverlayParams {
                        color: Color([255; 4]),
                    }),
                )
                .source(SourceRange::new(Tick::ZERO, Tick(20 * TICKS_PER_SECOND)).unwrap())
                .masks(vec![mask])
                .build(),
            ],
        });
        let project = Project::new(data).unwrap();
        let renderer =
            render::Renderer::new(&project, render::RenderSettings::from_project(&project))
                .unwrap();
        let viewport = Viewport::new(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 100.0,
            },
            [200, 100],
            1.0,
            [0.0, 0.0],
        )
        .unwrap();
        let geometry = renderer
            .mask_geometry(ItemId(1), "m", Tick::ZERO)
            .unwrap()
            .unwrap();
        let mut state = State::default();
        assert!(state.begin(
            &project,
            &renderer,
            &viewport,
            ItemId(1),
            "m",
            geometry,
            kind,
            press,
            Tick::ZERO,
        ));
        (state.drag.take().unwrap(), viewport)
    }

    #[test]
    fn move_snaps_the_box_to_the_canvas_centre_unless_disabled_or_bypassed() {
        // The box centre starts at (120, 50); dragging 18.5 px left lands it 1.5 px
        // short of the canvas centre line.
        for (enabled, bypass, centre_x) in [
            (true, false, 0.0),
            (false, false, 0.0075),
            (true, true, 0.0075),
        ] {
            let (mut drag, viewport) = gesture(Kind::Move, [120.0, 50.0]);
            let mask = drag
                .candidate(&viewport, [101.5, 50.0], enabled, bypass)
                .unwrap();
            assert!(
                (mask.center_x - centre_x).abs() < 1e-9,
                "{enabled} {bypass}: {}",
                mask.center_x
            );
            assert!(mask.center_y.abs() < 1e-9, "the vertical drag is zero");
            assert_eq!(
                (mask.width, mask.height, mask.scale, mask.rotation),
                (0.4, 0.4, 1.0, 0.0),
                "a move never touches the size or rotation"
            );
            if enabled && !bypass {
                assert!(drag.guides.contains(&PreviewGuide {
                    axis: PreviewAxis::Vertical,
                    position: 100.0,
                }));
            } else {
                assert!(drag.guides.is_empty());
            }
        }
    }

    #[test]
    fn resize_scales_about_the_centre_and_snaps_the_dragged_corner_per_axis() {
        // The bottom-right corner starts at (160, 70): the half box is 40 by 20 about the
        // centre (120, 50), so the half diagonal is hypot(40, 20).
        let half = 40f64.hypot(20.0);
        let (mut drag, viewport) = gesture(Kind::Resize, [160.0, 70.0]);
        // 3 px short of the canvas right edge: the snap pulls x to 200, y stays.
        let snapped = drag
            .candidate(&viewport, [197.0, 71.0], true, false)
            .unwrap();
        let expected = 80f64.hypot(21.0) / half;
        assert!(
            (snapped.scale - expected).abs() < 1e-9,
            "{} vs {expected}",
            snapped.scale
        );
        assert_eq!(
            drag.guides,
            vec![PreviewGuide {
                axis: PreviewAxis::Vertical,
                position: 200.0,
            }]
        );
        let free = drag
            .candidate(&viewport, [197.0, 71.0], false, false)
            .unwrap();
        let expected = 77f64.hypot(21.0) / half;
        assert!((free.scale - expected).abs() < 1e-9);
        assert!(drag.guides.is_empty());
        assert_eq!((free.center_x, free.rotation), (0.1, 0.0));
        // The model's scale range bounds a runaway drag.
        let huge = drag
            .candidate(&viewport, [1.0e9, 1.0e9], false, false)
            .unwrap();
        assert_eq!(huge.scale, SCALE_RANGE.1);
    }

    #[test]
    fn rotate_follows_the_pointer_angle_and_snaps_to_quarter_turns() {
        // The grip starts straight above the centre (120, 50); turning the pointer 87
        // degrees clockwise about it rotates the mask by 87, which snaps to 90.
        let (mut drag, viewport) = gesture(Kind::Rotate, [120.0, 18.0]);
        let turned = |degrees: f64| {
            let radians = degrees.to_radians();
            [120.0 + 32.0 * radians.sin(), 50.0 - 32.0 * radians.cos()]
        };
        let snapped = drag
            .candidate(&viewport, turned(87.0), true, false)
            .unwrap();
        assert_eq!(snapped.rotation, 90.0);
        assert_eq!(
            drag.guides,
            vec![PreviewGuide {
                axis: PreviewAxis::Vertical,
                position: 120.0,
            }]
        );
        let free = drag
            .candidate(&viewport, turned(87.0), false, false)
            .unwrap();
        assert!((free.rotation - 87.0).abs() < 1e-9, "{}", free.rotation);
        assert!(drag.guides.is_empty());
        // The shortest way round: 350 degrees is -10, never +350.
        let back = drag
            .candidate(&viewport, turned(350.0), false, false)
            .unwrap();
        assert!((back.rotation + 10.0).abs() < 1e-9, "{}", back.rotation);
        assert_eq!(
            (back.center_x, back.scale),
            (0.1, 1.0),
            "a rotation never touches the position or size"
        );
    }

    #[test]
    fn a_gesture_that_returns_to_its_origin_commits_nothing() {
        let (mut drag, viewport) = gesture(Kind::Move, [120.0, 50.0]);
        assert!(!drag.commit(), "nothing staged yet");
        drag.last = drag.candidate(&viewport, [120.0, 50.0], false, false);
        assert!(!drag.commit());
        drag.last = drag.candidate(&viewport, [130.0, 50.0], false, false);
        assert!(drag.commit());
    }

    #[test]
    fn the_handles_hit_the_grip_before_a_corner_and_nothing_inside() {
        let (drag, viewport) = gesture(Kind::Move, [120.0, 50.0]);
        let handles = Handles::new(&drag.geometry, &viewport).unwrap();
        assert_eq!(handles.hit(egui::pos2(120.0, 18.0)), Some(Kind::Rotate));
        assert_eq!(handles.hit(egui::pos2(160.0, 70.0)), Some(Kind::Resize));
        assert_eq!(
            handles.hit(egui::pos2(160.0 + 8.0, 70.0)),
            Some(Kind::Resize)
        );
        assert_eq!(handles.hit(egui::pos2(160.0 + 12.0, 70.0)), None);
        assert_eq!(handles.hit(egui::pos2(120.0, 50.0)), None);
    }
}
