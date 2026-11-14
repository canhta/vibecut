//! The matte refinement brush: one drag on the preview paints one add or remove stroke over the
//! selected clip's person matte.
//!
//! Points map through the compositor's own placement (`Renderer::frame_point`), so a stroke is
//! stored in decoded-frame units and lands on what the preview shows, under crop, fit and
//! transform. The release publishes the whole stroke as one action, hence one undo step.
use super::{Action, Context, state::MatteBrush};
use crate::egui::{self, CursorIcon, Pos2};
use project::MatteStroke;
use time::Tick;

/// The Size control's range and default, read from the reference (Remove BG, Custom removal).
pub(crate) const SIZE_MIN: u32 = 1;
pub(crate) const SIZE_MAX: u32 = 100;
pub(crate) const SIZE_DEFAULT: u32 = 20;

/// Tip radius per Size unit, as a fraction of the frame's shorter side. Derived from one
/// measurement: at Size 20 the reference's cursor circle is 14.2 logical px across on a preview
/// frame 230 px high, a diameter of 6.17 % of the shorter side, so Size 20 is a radius of
/// 3.09 %; the mapping is taken as linear through zero (only Size 20 was measured).
const RADIUS_PER_SIZE: f32 = 0.00155;

/// Tip radius of a stroke at `size`, as a fraction of the decoded frame's shorter side. The
/// stored `MatteStroke::radius` is this value, so the stored shape is unchanged.
pub(super) fn radius(size: u32) -> f32 {
    size.clamp(SIZE_MIN, SIZE_MAX) as f32 * RADIUS_PER_SIZE
}

/// The open stroke: each point in decoded-frame units with the screen position it was read at.
pub(in crate::editor) type Trail = Vec<([f32; 2], Pos2)>;

/// Continue the drag in progress or start one; a release publishes the stroke.
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    trail: &mut Option<Trail>,
    brush: &MatteBrush,
    size: u32,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
    frame: egui::Rect,
    canvas: &egui::Response,
    at: Tick,
    actions: &mut Vec<Action>,
) {
    let tip = radius(size);
    let tip_px = tip * frame.width().min(frame.height());
    if canvas.hovered() {
        // The system cursor gives way to a circle the size of the tip.
        ui.ctx().set_cursor_icon(CursorIcon::None);
        if let Some(pointer) = ui.input(|i| i.pointer.hover_pos()) {
            let painter = ui.painter().with_clip_rect(frame);
            painter.circle(
                pointer,
                tip_px,
                egui::Color32::from_white_alpha(24),
                egui::Stroke::new(1.0, egui::Color32::from_white_alpha(200)),
            );
        }
    }
    let project = cx.staged_project();
    let dimensions = &project.data().canvas;
    let (Some(viewport), Some(renderer)) = (
        super::transform::viewport(frame, [dimensions.width, dimensions.height]),
        super::mask_path::renderer(cx),
    ) else {
        *trail = None;
        return;
    };
    let pointer = ui.input(|i| i.pointer.interact_pos().or_else(|| i.pointer.latest_pos()));
    let pressed = ui.input(|i| i.pointer.primary_pressed());
    let down = ui.input(|i| i.pointer.primary_down());
    if pressed && canvas.is_pointer_button_down_on() {
        *trail = Some(Vec::new());
    }
    if down {
        let Some(points) = trail.as_mut() else {
            return;
        };
        if let Some(pointer) = pointer
            && points.len() < MatteStroke::MAX_POINTS
            && let Ok(Some((point, _))) = renderer.frame_point(
                &viewport,
                brush.item,
                at,
                [f64::from(pointer.x), f64::from(pointer.y)],
            )
        {
            let point = [point[0] as f32, point[1] as f32];
            if points.last().map(|(last, _)| *last) != Some(point) {
                points.push((point, pointer));
            }
        }
        let width = 2.0 * tip_px;
        let painter = ui.painter().with_clip_rect(frame);
        let line: Vec<Pos2> = points.iter().map(|(_, screen)| *screen).collect();
        painter.add(egui::Shape::line(
            line,
            egui::Stroke::new(width, crate::kit::accent(cx.dark).gamma_multiply(0.4)),
        ));
        ui.ctx().request_repaint();
    } else if let Some(points) = trail.take() {
        let Some(item) = project
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter())
            .find(|candidate| candidate.id == brush.item)
        else {
            return;
        };
        let (Ok(source), false) = (item.source_time_at(at), points.is_empty()) else {
            return;
        };
        actions.push(Action::PaintMatteStroke {
            item: brush.item,
            stroke: MatteStroke {
                mode: brush.mode,
                at: source,
                radius: tip,
                points: points.into_iter().map(|(point, _)| point).collect(),
            },
        });
    }
}
