//! Point and tangent editing for one selected item's closed cubic mask path.
//!
//! Coordinates stay normalized to the evaluated mask box; every hit test and
//! drag maps through the render-owned mask affine, so handles stay on the
//! rendered contour under mask and item rotation and scale.

use super::{Action, Context};
use crate::{
    egui::{self, CursorIcon, Pos2, Sense, Vec2},
    kit,
    selection::MaskPointSelection,
};
use project::{ItemId, MaskPath, MaskShape};
use render::viewport::{MaskGeometry, Viewport};
use time::Tick;

/// A segment click within this many logical pixels inserts one point.
const SEGMENT_HIT: f64 = 6.0;
/// Samples per cubic when locating the nearest segment parameter.
const SAMPLES: usize = 64;

#[derive(Default)]
pub(in crate::editor) struct State {
    drag: Option<Drag>,
}

impl State {
    /// Abandon a mask gesture; the caller discards its staged candidate.
    pub(in crate::editor) fn cancel(&mut self) -> bool {
        self.drag.take().is_some()
    }
}

struct Target {
    item: ItemId,
    mask: String,
    path: MaskPath,
    geometry: MaskGeometry,
}

#[derive(Clone, Copy)]
enum Kind {
    Points,
    Tangent { point: usize, out: bool },
}

struct Drag {
    item: ItemId,
    mask: String,
    start: MaskPath,
    origin: [f64; 2],
    kind: Kind,
    /// Point indices moved together, captured at the press.
    points: Vec<usize>,
    last: Option<MaskPath>,
}

/// The closed path mask the player can edit: the single selected item's first
/// active path mask with at least three anchors.
fn target(
    project: &project::Project,
    renderer: &render::Renderer<'_>,
    item: ItemId,
    at: Tick,
) -> Option<Target> {
    let source = project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == item)?;
    let mask = source
        .masks
        .iter()
        .find(|mask| mask.active() && matches!(mask.shape, MaskShape::Path { .. }))?;
    let MaskShape::Path { path } = &mask.shape else {
        return None;
    };
    if !path.closed || path.vertices.len() < 3 {
        return None;
    }
    Some(Target {
        item,
        mask: mask.id.clone(),
        path: path.clone(),
        geometry: renderer.mask_geometry(item, &mask.id, at).ok().flatten()?,
    })
}

pub(super) fn show(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
    viewport: &Viewport,
    frame: egui::Rect,
    canvas: &egui::Response,
    at: Tick,
    actions: &mut Vec<Action>,
) -> bool {
    let Some(target) = crate::editor::context::single(cx.selection)
        .and_then(|item| target(cx.staged_project(), &renderer(cx)?, item, at))
    else {
        state.drag = None;
        return false;
    };
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let pressed = ui.input(|i| i.pointer.primary_pressed());
    let down = ui.input(|i| i.pointer.primary_down());
    let shift = ui.input(|i| i.modifiers.shift);
    let selected = |point: usize| {
        cx.mask_points.contains(&MaskPointSelection {
            item: target.item,
            mask: target.mask.clone(),
            point,
        })
    };

    if let Some(mut drag) = state.drag.take() {
        if down {
            if let Some(pointer) = pointer
                && let Ok(local) = target
                    .geometry
                    .inverse(viewport, [f64::from(pointer.x), f64::from(pointer.y)])
                && let Some(path) = moved(&drag, local)
                && drag.last.as_ref() != Some(&path)
            {
                drag.last = Some(path.clone());
                actions.push(Action::MaskPathPreview {
                    item: drag.item,
                    mask: drag.mask.clone(),
                    path,
                });
            }
            state.drag = Some(drag);
        } else {
            actions.push(Action::MaskPathEnd {
                commit: drag.last.as_ref().is_some_and(|path| path != &drag.start),
            });
        }
        if let Some(pointer) = pointer {
            paint(ui, &target, viewport, &selected, cx.dark, Some(pointer));
        } else {
            paint(ui, &target, viewport, &selected, cx.dark, None);
        }
        return true;
    }

    paint(ui, &target, viewport, &selected, cx.dark, None);
    let Some(pointer) = pointer else {
        return false;
    };
    if !pressed || !(canvas.is_pointer_button_down_on() || frame.contains(pointer)) {
        return false;
    }
    let screen = [f64::from(pointer.x), f64::from(pointer.y)];
    let mut handles = Vec::new();
    for (index, vertex) in target.path.vertices.iter().enumerate() {
        if let Ok(position) = target.geometry.point(viewport, vertex.p) {
            handles.push((Kind::Points, index, position));
        }
        if selected(index) {
            for (out, tangent) in [(true, vertex.t_out), (false, vertex.t_in)] {
                let endpoint = [vertex.p[0] + tangent[0], vertex.p[1] + tangent[1]];
                if let Ok(position) = target.geometry.point(viewport, endpoint) {
                    handles.push((Kind::Tangent { point: index, out }, index, position));
                }
            }
        }
    }
    let nearest = handles
        .iter()
        .map(|(kind, index, position)| {
            (
                (screen[0] - position[0]).hypot(screen[1] - position[1]),
                *kind,
                *index,
            )
        })
        .filter(|(distance, _, _)| *distance <= f64::from(kit::size::TRANSFORM_HIT))
        .min_by(|a, b| a.0.total_cmp(&b.0));
    if let Some((_, kind, index)) = nearest {
        match kind {
            Kind::Tangent { point, out } => {
                if !selected(point) {
                    actions.push(Action::MaskPointSelect {
                        target: point_target(&target, point),
                        toggle: false,
                    });
                }
                if let Ok(origin) = target.geometry.inverse(viewport, screen) {
                    state.drag = Some(Drag {
                        item: target.item,
                        mask: target.mask.clone(),
                        start: target.path.clone(),
                        origin,
                        kind: Kind::Tangent { point, out },
                        points: Vec::new(),
                        last: None,
                    });
                }
            }
            Kind::Points => {
                if shift {
                    actions.push(Action::MaskPointSelect {
                        target: point_target(&target, index),
                        toggle: true,
                    });
                } else {
                    let points = if selected(index) {
                        selection(&cx.mask_points, &target)
                    } else {
                        actions.push(Action::MaskPointSelect {
                            target: point_target(&target, index),
                            toggle: false,
                        });
                        vec![index]
                    };
                    if let Ok(origin) = target.geometry.inverse(viewport, screen) {
                        state.drag = Some(Drag {
                            item: target.item,
                            mask: target.mask.clone(),
                            start: target.path.clone(),
                            origin,
                            kind: Kind::Points,
                            points,
                            last: None,
                        });
                    }
                }
            }
        }
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        return true;
    }
    if let Some((segment, t, _)) = nearest_segment(&target, viewport, screen) {
        actions.push(Action::MaskPointAdd {
            item: target.item,
            mask: target.mask.clone(),
            segment,
            t,
        });
        return true;
    }
    if !cx.mask_points.is_empty() && frame.contains(pointer) {
        actions.push(Action::MaskPointClear);
        return true;
    }
    false
}

fn renderer<'a>(cx: &'a Context<'_>) -> Option<render::Renderer<'a>> {
    let project = cx.staged_project();
    let settings = cx
        .playback
        .render_state()
        .and_then(|state| state.settings(project))
        .ok()?;
    render::Renderer::new(project, settings).ok()
}

fn point_target(target: &Target, point: usize) -> MaskPointSelection {
    MaskPointSelection {
        item: target.item,
        mask: target.mask.clone(),
        point,
    }
}

fn selection(
    selected: &std::collections::BTreeSet<MaskPointSelection>,
    target: &Target,
) -> Vec<usize> {
    selected
        .iter()
        .filter(|point| point.item == target.item && point.mask == target.mask)
        .map(|point| point.point)
        .filter(|point| *point < target.path.vertices.len())
        .collect()
}

/// Absolute candidate from the entry path; a drag never accumulates.
fn moved(drag: &Drag, local: [f64; 2]) -> Option<MaskPath> {
    let mut path = drag.start.clone();
    match drag.kind {
        Kind::Points => {
            let delta = [local[0] - drag.origin[0], local[1] - drag.origin[1]];
            for index in &drag.points {
                let vertex = path.vertices.get_mut(*index)?;
                vertex.p = [clamp(vertex.p[0] + delta[0]), clamp(vertex.p[1] + delta[1])];
            }
        }
        Kind::Tangent { point, out } => {
            let vertex = path.vertices.get_mut(point)?;
            let offset = [clamp(local[0] - vertex.p[0]), clamp(local[1] - vertex.p[1])];
            if out {
                vertex.t_out = offset;
            } else {
                vertex.t_in = offset;
            }
        }
    }
    Some(path)
}

fn clamp(value: f64) -> f64 {
    value.clamp(-100.0, 100.0)
}

fn cubic(p: [[f64; 2]; 4], t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    std::array::from_fn(|k| {
        u * u * u * p[0][k]
            + 3.0 * u * u * t * p[1][k]
            + 3.0 * u * t * t * p[2][k]
            + t * t * t * p[3][k]
    })
}

fn controls(path: &MaskPath, segment: usize) -> [[f64; 2]; 4] {
    let next = (segment + 1) % path.vertices.len();
    let a = &path.vertices[segment];
    let b = &path.vertices[next];
    [
        a.p,
        [a.p[0] + a.t_out[0], a.p[1] + a.t_out[1]],
        [b.p[0] + b.t_in[0], b.p[1] + b.t_in[1]],
        b.p,
    ]
}

/// Nearest cubic parameter in logical screen pixels, or `None` beyond the
/// insertion radius. The closing edge is a normal segment.
fn nearest_segment(
    target: &Target,
    viewport: &Viewport,
    screen: [f64; 2],
) -> Option<(usize, f64, f64)> {
    let distance = |segment: usize, t: f64| -> Option<f64> {
        let point = target
            .geometry
            .point(viewport, cubic(controls(&target.path, segment), t))
            .ok()?;
        Some((point[0] - screen[0]).hypot(point[1] - screen[1]))
    };
    let mut best: Option<(usize, f64, f64)> = None;
    for segment in 0..target.path.vertices.len() {
        for sample in 0..=SAMPLES {
            let t = sample as f64 / SAMPLES as f64;
            let Some(value) = distance(segment, t) else {
                continue;
            };
            if best.is_none_or(|(_, _, nearest)| value < nearest) {
                best = Some((segment, t, value));
            }
        }
    }
    let (segment, t, _) = best?;
    let step = 1.0 / SAMPLES as f64;
    let mut low = (t - step).max(0.0);
    let mut high = (t + step).min(1.0);
    for _ in 0..24 {
        let a = low + (high - low) / 3.0;
        let b = high - (high - low) / 3.0;
        match (distance(segment, a), distance(segment, b)) {
            (Some(a), Some(b)) if a < b => high = b,
            (Some(_), Some(_)) => low = a,
            _ => break,
        }
    }
    // The engine splits only strictly inside a segment (2^-20..=1-2^-20).
    let t = ((low + high) / 2.0).clamp(1.0 / 1_048_576.0, 1.0 - 1.0 / 1_048_576.0);
    let value = distance(segment, t)?;
    (value <= SEGMENT_HIT).then_some((segment, t, value))
}

fn paint(
    ui: &egui::Ui,
    target: &Target,
    viewport: &Viewport,
    selected: &impl Fn(usize) -> bool,
    dark: bool,
    pointer: Option<Pos2>,
) {
    let painter = ui.painter();
    let stroke = egui::Stroke::new(kit::size::TRANSFORM_STROKE, kit::accent(dark));
    let mut curve = Vec::with_capacity(target.path.vertices.len() * 24);
    for segment in 0..target.path.vertices.len() {
        let control = controls(&target.path, segment);
        for step in 0..24 {
            let t = step as f64 / 24.0;
            if let Ok([x, y]) = target.geometry.point(viewport, cubic(control, t)) {
                curve.push(egui::pos2(x as f32, y as f32));
            }
        }
    }
    if let Some(first) = curve.first().copied() {
        curve.push(first);
        painter.add(egui::Shape::line(curve, stroke));
    }
    for (index, vertex) in target.path.vertices.iter().enumerate() {
        let hit = selected(index);
        if hit {
            for tangent in [vertex.t_out, vertex.t_in] {
                let endpoint = [vertex.p[0] + tangent[0], vertex.p[1] + tangent[1]];
                if let (Ok(anchor), Ok(end)) = (
                    target.geometry.point(viewport, vertex.p),
                    target.geometry.point(viewport, endpoint),
                ) {
                    let anchor = egui::pos2(anchor[0] as f32, anchor[1] as f32);
                    let end = egui::pos2(end[0] as f32, end[1] as f32);
                    painter.line_segment([anchor, end], stroke);
                    painter.circle_filled(end, kit::size::TRANSFORM_HANDLE / 2.0, kit::panel(dark));
                    painter.circle_stroke(end, kit::size::TRANSFORM_HANDLE / 2.0, stroke);
                }
            }
        }
        if let Ok([x, y]) = target.geometry.point(viewport, vertex.p) {
            let position = egui::pos2(x as f32, y as f32);
            let radius = kit::size::TRANSFORM_HANDLE / 2.0;
            painter.circle_filled(
                position,
                radius,
                if hit {
                    kit::accent(dark)
                } else {
                    kit::panel(dark)
                },
            );
            painter.circle_stroke(position, radius, stroke);
        }
    }
    if let Some(pointer) = pointer
        && nearest_segment(
            target,
            viewport,
            [f64::from(pointer.x), f64::from(pointer.y)],
        )
        .is_some()
    {
        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
    }
}

/// Register one stable automation target for each anchor and tangent handle.
pub(super) fn remember(
    ui: &mut egui::Ui,
    cx: &Context<'_>,
    viewport: &Viewport,
    at: Tick,
    actions: &mut Vec<Action>,
) {
    let Some(target) = crate::editor::context::single(cx.selection)
        .and_then(|item| target(cx.staged_project(), &renderer(cx)?, item, at))
    else {
        return;
    };
    for (index, vertex) in target.path.vertices.iter().enumerate() {
        let Ok([x, y]) = target.geometry.point(viewport, vertex.p) else {
            continue;
        };
        let position = egui::pos2(x as f32, y as f32);
        let response = ui.interact(
            egui::Rect::from_center_size(position, Vec2::splat(kit::size::TRANSFORM_HIT * 2.0)),
            ui.make_persistent_id(("preview.mask.point", index)),
            Sense::click_and_drag(),
        );
        let selected = cx.mask_points.contains(&MaskPointSelection {
            item: target.item,
            mask: target.mask.clone(),
            point: index,
        });
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::Role::CheckBox,
                true,
                selected,
                crate::i18n::text(cx.locale, crate::i18n::Text::MaskPath),
            )
        });
        actions.push(Action::Remember {
            id: format!("preview.mask.point.{index}"),
            icon: None,
            tooltip: None,
            response,
        });
    }
}
