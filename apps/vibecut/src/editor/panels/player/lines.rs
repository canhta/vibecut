//! Preview rulers and custom guide lines: the stored model, ruler ticks, painting
//! and the guide drag.

use super::{
    Action, Context, State,
    view::{menu, remember},
};
use crate::{
    Error, Result,
    egui::{self, FontId, Pos2, Rect, Sense},
    i18n::Text,
    kit,
};
use serde::{Deserialize, Serialize};

/// Most guide lines one axis holds.
pub const LINES_MAX: usize = 16;
/// Farthest a guide may sit from the canvas centre, in canvas pixels.
pub const LINE_LIMIT: i32 = 100_000;

/// The axis a guide line runs along: a vertical line crosses the x ruler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineAxis {
    Vertical,
    Horizontal,
}

/// The rulers and the custom guide lines of the preview. Positions are canvas
/// pixels from the canvas centre: x grows to the right and y grows upward, the
/// way the rulers label them. The app paints them only, so export never sees them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuideLines {
    /// The top and left rulers are drawn.
    pub rulers: bool,
    /// The guide lines are drawn; adding one shows them again.
    pub shown: bool,
    /// Locked guide lines cannot be dragged.
    pub locked: bool,
    /// Vertical lines, as x positions.
    pub vertical: Vec<i32>,
    /// Horizontal lines, as y positions.
    pub horizontal: Vec<i32>,
}

impl Default for GuideLines {
    fn default() -> Self {
        Self {
            rulers: false,
            shown: true,
            locked: false,
            vertical: Vec::new(),
            horizontal: Vec::new(),
        }
    }
}

impl GuideLines {
    pub fn is_empty(&self) -> bool {
        self.vertical.is_empty() && self.horizontal.is_empty()
    }

    pub fn positions(&self, axis: LineAxis) -> &[i32] {
        match axis {
            LineAxis::Vertical => &self.vertical,
            LineAxis::Horizontal => &self.horizontal,
        }
    }

    fn positions_mut(&mut self, axis: LineAxis) -> &mut Vec<i32> {
        match axis {
            LineAxis::Vertical => &mut self.vertical,
            LineAxis::Horizontal => &mut self.horizontal,
        }
    }

    pub fn validate(&self) -> Result<()> {
        for axis in [LineAxis::Vertical, LineAxis::Horizontal] {
            let positions = self.positions(axis);
            if positions.len() > LINES_MAX
                || positions
                    .iter()
                    .any(|position| position.unsigned_abs() > LINE_LIMIT.unsigned_abs())
            {
                return Err(Error::Invalid("guide lines"));
            }
        }
        Ok(())
    }

    /// These lines with the rulers on or off.
    pub fn with_rulers(&self, rulers: bool) -> Self {
        Self {
            rulers,
            ..self.clone()
        }
    }

    /// These lines shown or hidden.
    pub fn with_shown(&self, shown: bool) -> Self {
        Self {
            shown,
            ..self.clone()
        }
    }

    /// These lines locked or unlocked.
    pub fn with_locked(&self, locked: bool) -> Self {
        Self {
            locked,
            ..self.clone()
        }
    }

    /// One more line at the canvas centre, shown. Refused at `LINES_MAX`.
    pub fn with_added(&self, axis: LineAxis) -> Result<Self> {
        self.with_added_at(axis, 0)
    }

    /// One more line at an absolute position, shown. Refused at `LINES_MAX` or
    /// beyond `LINE_LIMIT`. A lock does not refuse it: the lock stops moving lines.
    pub fn with_added_at(&self, axis: LineAxis, position: i32) -> Result<Self> {
        let mut next = self.clone();
        next.shown = true;
        next.positions_mut(axis).push(position);
        next.validate()?;
        Ok(next)
    }

    /// One line moved to an absolute position. Refused while locked, for an
    /// unknown line, or beyond `LINE_LIMIT`.
    pub fn with_moved(&self, axis: LineAxis, index: usize, position: i32) -> Result<Self> {
        if self.locked {
            return Err(Error::Invalid("locked guide lines"));
        }
        let mut next = self.clone();
        *next
            .positions_mut(axis)
            .get_mut(index)
            .ok_or(Error::Invalid("guide line"))? = position;
        next.validate()?;
        Ok(next)
    }

    /// No lines, shown and unlocked; the rulers keep their state.
    pub fn cleared(&self) -> Self {
        Self {
            rulers: self.rulers,
            ..Self::default()
        }
    }
}

/// The guide line a drag is moving, or the new one pulled out of a ruler; it is
/// published once when the drag ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) struct LineDrag {
    axis: LineAxis,
    /// The line being moved; `None` for a new line pulled out of a ruler.
    index: Option<usize>,
    position: i32,
    /// A ruler pull creates its line only when it ends over the viewer.
    inside: bool,
}

/// One ruler tick, in canvas units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Mark {
    pub value: f64,
    /// A labelled tick; ten ticks make one major step.
    pub major: bool,
}

/// Canvas units between labelled ticks on a ruler `length` screen pixels long: the
/// smallest 1, 2, 5 or 7 times a power of ten that leaves at most
/// `kit::size::PLAYER_RULER_MAX_SPANS` labelled spans along the ruler.
pub(super) fn major_step(scale: f32, length: f32) -> Option<f64> {
    if !scale.is_finite() || scale <= 0.0 || !length.is_finite() || length <= 0.0 {
        return None;
    }
    let wanted =
        f64::from(length) / f64::from(scale) / f64::from(kit::size::PLAYER_RULER_MAX_SPANS);
    let decade = 10f64.powf(wanted.log10().floor());
    [1.0, 2.0, 5.0, 7.0, 10.0]
        .into_iter()
        .map(|factor| decade * factor)
        .find(|step| *step >= wanted)
}

/// Every tick from `from` to `to` canvas units, ten per major step.
pub(super) fn marks(from: f64, to: f64, step: f64) -> Vec<Mark> {
    if !from.is_finite() || !to.is_finite() || !step.is_finite() || step <= 0.0 || to < from {
        return Vec::new();
    }
    let first = (from / step * 10.0).ceil() as i64;
    let last = (to / step * 10.0).floor() as i64;
    (first..=last)
        .take(4096)
        .map(|k| Mark {
            value: k as f64 * step / 10.0,
            major: k.rem_euclid(10) == 0,
        })
        .collect()
}

/// The ruler caption of a major tick: whole canvas units.
pub(super) fn label(value: f64) -> String {
    format!("{}", value.round() as i64)
}

/// Where a canvas position lands on screen along its axis.
fn screen(origin: Pos2, scale: f32, axis: LineAxis, position: f64) -> f32 {
    match axis {
        LineAxis::Vertical => origin.x + position as f32 * scale,
        LineAxis::Horizontal => origin.y - position as f32 * scale,
    }
}

/// The canvas position under a pointer coordinate along one axis.
fn canvas_position(origin: Pos2, scale: f32, axis: LineAxis, pointer: Pos2) -> i32 {
    let position = match axis {
        LineAxis::Vertical => (pointer.x - origin.x) / scale,
        LineAxis::Horizontal => (origin.y - pointer.y) / scale,
    };
    position
        .round()
        .clamp(-(LINE_LIMIT as f32), LINE_LIMIT as f32) as i32
}

/// The two ends of a guide line across the viewer content, and the pointer band around it.
fn line_and_hit(content: Rect, axis: LineAxis, at: f32) -> ([Pos2; 2], Rect) {
    let half = kit::size::GUIDE_LINE_HIT / 2.0;
    match axis {
        LineAxis::Vertical => (
            [
                egui::pos2(at, content.top()),
                egui::pos2(at, content.bottom()),
            ],
            Rect::from_x_y_ranges(at - half..=at + half, content.y_range()),
        ),
        LineAxis::Horizontal => (
            [
                egui::pos2(content.left(), at),
                egui::pos2(content.right(), at),
            ],
            Rect::from_x_y_ranges(content.x_range(), at - half..=at + half),
        ),
    }
}

/// Draws the guide lines and the rulers over the stage, handles a guide drag and the
/// pull of a new guide out of a ruler. `stage` is the whole viewer: the rulers run along
/// its top and left edges and the guides to its other edges, past the canvas.
/// `origin` is the canvas centre on screen and `scale` the screen pixels per canvas pixel.
pub(super) fn show(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
    stage: Rect,
    origin: Pos2,
    scale: f32,
    actions: &mut Vec<Action>,
) {
    let lines = &cx.preferences.preview_lines;
    let thickness = if lines.rulers {
        kit::size::PLAYER_RULER
    } else {
        0.0
    };
    let content = Rect::from_min_max(stage.min + egui::vec2(thickness, thickness), stage.max);
    let stroke = egui::Stroke::new(kit::size::GUIDE_LINE, kit::guide_line(cx.dark));
    if !lines.rulers || scale <= 0.0 {
        // A pull out of a ruler ends with its ruler.
        state.line_drag = state.line_drag.filter(|drag| drag.index.is_some());
    }
    if lines.shown && scale > 0.0 {
        let painter = ui.painter().with_clip_rect(content);
        for (axis, name) in [(LineAxis::Vertical, "v"), (LineAxis::Horizontal, "h")] {
            for (index, saved) in lines.positions(axis).iter().enumerate() {
                let position = state
                    .line_drag
                    .filter(|drag| drag.axis == axis && drag.index == Some(index))
                    .map_or(*saved, |drag| drag.position);
                let at = screen(origin, scale, axis, f64::from(position));
                let (line, hit) = line_and_hit(content, axis, at);
                painter.line_segment(line, stroke);
                let hit = hit.intersect(content);
                if !hit.is_positive() {
                    continue;
                }
                let sense = if lines.locked {
                    Sense::hover()
                } else {
                    Sense::drag()
                };
                let response = ui.interact(
                    hit,
                    ui.make_persistent_id(("preview.lines", name, index)),
                    sense,
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::Button, !lines.locked, "Guide")
                });
                if !lines.locked {
                    if response.hovered() || response.dragged() {
                        ui.ctx().set_cursor_icon(match axis {
                            LineAxis::Vertical => egui::CursorIcon::ResizeHorizontal,
                            LineAxis::Horizontal => egui::CursorIcon::ResizeVertical,
                        });
                    }
                    if response.dragged()
                        && let Some(pointer) = response.interact_pointer_pos()
                    {
                        state.line_drag = Some(LineDrag {
                            axis,
                            index: Some(index),
                            position: canvas_position(origin, scale, axis, pointer),
                            inside: true,
                        });
                    }
                    if response.drag_stopped()
                        && let Some(drag) = state.line_drag.take()
                        && drag.position != *saved
                    {
                        actions.push(Action::MoveGuide {
                            axis,
                            index,
                            position: drag.position,
                        });
                    }
                }
                remember(
                    cx,
                    actions,
                    &format!("preview.guides.{name}.{index}"),
                    response,
                );
            }
        }
    } else {
        state.line_drag = state.line_drag.filter(|drag| drag.index.is_none());
    }
    if lines.rulers && scale > 0.0 {
        ruler_pull(state, cx, ui, stage, origin, scale, actions);
        paint_rulers(ui, cx.dark, stage, origin, scale);
    }
}

/// A drag that starts on a ruler pulls a new guide out of it: the top ruler makes a
/// horizontal line and the left ruler a vertical one. The line follows the pointer and is
/// published as `preview.guides.add_*` at the drop position when the drag ends over the
/// viewer.
fn ruler_pull(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
    stage: Rect,
    origin: Pos2,
    scale: f32,
    actions: &mut Vec<Action>,
) {
    let thickness = kit::size::PLAYER_RULER;
    let content = Rect::from_min_max(stage.min + egui::vec2(thickness, thickness), stage.max);
    let stroke = egui::Stroke::new(kit::size::GUIDE_LINE, kit::guide_line(cx.dark));
    let top = Rect::from_min_max(
        egui::pos2(content.left(), stage.top()),
        egui::pos2(stage.right(), content.top()),
    );
    let left = Rect::from_min_max(
        egui::pos2(stage.left(), content.top()),
        egui::pos2(content.left(), stage.bottom()),
    );
    for (strip, axis, id, cursor) in [
        (
            top,
            LineAxis::Horizontal,
            "preview.rulers.top",
            egui::CursorIcon::ResizeVertical,
        ),
        (
            left,
            LineAxis::Vertical,
            "preview.rulers.left",
            egui::CursorIcon::ResizeHorizontal,
        ),
    ] {
        if !strip.is_positive() {
            continue;
        }
        let response = ui.interact(strip, ui.make_persistent_id(id), Sense::drag());
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(cursor);
        }
        if response.dragged()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            state.line_drag = Some(LineDrag {
                axis,
                index: None,
                position: canvas_position(origin, scale, axis, pointer),
                inside: content.contains(pointer),
            });
        }
        if response.drag_stopped()
            && let Some(drag) = state.line_drag.take()
            && drag.index.is_none()
            && drag.inside
        {
            actions.push(Action::AddGuide {
                axis,
                position: drag.position,
            });
        }
        remember(cx, actions, id, response);
    }
    if let Some(drag) = state.line_drag
        && drag.index.is_none()
        && drag.inside
    {
        let at = screen(origin, scale, drag.axis, f64::from(drag.position));
        let (line, _) = line_and_hit(content, drag.axis, at);
        ui.painter()
            .with_clip_rect(content)
            .line_segment(line, stroke);
    }
}

/// Draws the top and left rulers over the stage edges.
fn paint_rulers(ui: &egui::Ui, dark: bool, stage: Rect, origin: Pos2, scale: f32) {
    let thickness = kit::size::PLAYER_RULER;
    // Each ruler picks its own step from its own length.
    let (Some(top_step), Some(left_step)) = (
        major_step(scale, stage.width() - thickness),
        major_step(scale, stage.height() - thickness),
    ) else {
        return;
    };
    let painter = ui.painter().with_clip_rect(stage);
    let top = Rect::from_min_size(stage.min, egui::vec2(stage.width(), thickness));
    let left = Rect::from_min_max(
        egui::pos2(stage.left(), stage.top() + thickness),
        egui::pos2(stage.left() + thickness, stage.bottom()),
    );
    for strip in [top, left] {
        painter.rect_filled(strip, 0.0, kit::panel(dark));
    }
    let hairline = egui::Stroke::new(kit::space::HAIRLINE, kit::border(dark));
    let major = egui::Stroke::new(kit::space::HAIRLINE, kit::disabled(dark));
    let font = FontId::new(kit::type_size::BADGE, kit::label_font().family);
    let text = kit::secondary(dark);
    // The top ruler reads x from the canvas centre, the left ruler reads y upward.
    painter.hline(
        top.left() + thickness..=top.right(),
        top.bottom() - kit::space::HAIRLINE / 2.0,
        hairline,
    );
    painter.vline(
        left.right() - kit::space::HAIRLINE / 2.0,
        left.y_range(),
        hairline,
    );
    let units = |low: f32, high: f32, axis: LineAxis, step: f64| {
        let (a, b) = match axis {
            LineAxis::Vertical => ((low - origin.x) / scale, (high - origin.x) / scale),
            LineAxis::Horizontal => ((origin.y - high) / scale, (origin.y - low) / scale),
        };
        marks(f64::from(a), f64::from(b), step)
    };
    for mark in units(
        top.left() + thickness,
        top.right(),
        LineAxis::Vertical,
        top_step,
    ) {
        let x = screen(origin, scale, LineAxis::Vertical, mark.value);
        let (height, stroke) = if mark.major {
            (kit::size::PLAYER_RULER_TICK_MAJOR, major)
        } else {
            (kit::size::PLAYER_RULER_TICK, hairline)
        };
        painter.vline(x, top.bottom() - height..=top.bottom(), stroke);
        if mark.major {
            painter.text(
                egui::pos2(x + kit::size::PLAYER_RULER_LABEL_GAP, top.top()),
                egui::Align2::LEFT_TOP,
                label(mark.value),
                font.clone(),
                text,
            );
        }
    }
    for mark in units(left.top(), left.bottom(), LineAxis::Horizontal, left_step) {
        let y = screen(origin, scale, LineAxis::Horizontal, mark.value);
        let (height, stroke) = if mark.major {
            (kit::size::PLAYER_RULER_TICK_MAJOR, major)
        } else {
            (kit::size::PLAYER_RULER_TICK, hairline)
        };
        painter.hline(left.right() - height..=left.right(), y, stroke);
        if mark.major {
            // Rotated a quarter turn so the caption reads upward, starting past the tick.
            let galley = painter.layout_no_wrap(label(mark.value), font.clone(), text);
            let size = galley.size();
            let shape = egui::epaint::TextShape::new(
                egui::pos2(
                    left.left() + (thickness - size.y) / 2.0,
                    y - kit::size::PLAYER_RULER_LABEL_GAP,
                ),
                galley,
                text,
            )
            .with_angle(-std::f32::consts::FRAC_PI_2);
            painter.add(shape);
        }
    }
}

/// The `Guides and rulers` submenu of the player menu. Every row is the registered
/// action of the same name, so the menu, the keys and agents share one path.
pub(super) fn submenu(ui: &mut egui::Ui, cx: &Context<'_>, actions: &mut Vec<Action>) {
    let lines = &cx.preferences.preview_lines;
    let has_lines = !lines.is_empty();
    let response = ui.menu_button(cx.tr(Text::GuidesAndRulers), |ui| {
        menu(
            ui,
            cx,
            actions,
            "preview.menu.guides.add_horizontal",
            Text::GuidesAddHorizontal,
            true,
            "preview.guides.add_horizontal",
            None,
        );
        menu(
            ui,
            cx,
            actions,
            "preview.menu.guides.add_vertical",
            Text::GuidesAddVertical,
            true,
            "preview.guides.add_vertical",
            None,
        );
        menu(
            ui,
            cx,
            actions,
            "preview.menu.guides.lock",
            if lines.locked {
                Text::GuidesUnlock
            } else {
                Text::GuidesLock
            },
            has_lines,
            "preview.guides.lock",
            None,
        );
        menu(
            ui,
            cx,
            actions,
            "preview.menu.guides.visible",
            if lines.shown {
                Text::GuidesHide
            } else {
                Text::GuidesShow
            },
            has_lines,
            "preview.guides.visible",
            None,
        );
        menu(
            ui,
            cx,
            actions,
            "preview.menu.guides.clear",
            Text::GuidesClear,
            has_lines,
            "preview.guides.clear",
            None,
        );
        menu(
            ui,
            cx,
            actions,
            "preview.menu.rulers",
            if lines.rulers {
                Text::RulersHide
            } else {
                Text::RulersShow
            },
            true,
            "preview.rulers",
            None,
        );
    });
    remember(cx, actions, "preview.menu.guides", response.response);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn a_new_guide_sits_at_the_centre_shows_the_lines_and_stops_at_the_limit() {
        let hidden = GuideLines::default().with_shown(false);
        let added = hidden.with_added(LineAxis::Horizontal).unwrap();
        assert_eq!(added.horizontal, vec![0]);
        assert!(added.vertical.is_empty());
        assert!(added.shown, "adding a guide shows the guides again");
        let mut full = GuideLines::default();
        for _ in 0..LINES_MAX {
            full = full.with_added(LineAxis::Vertical).unwrap();
        }
        assert!(full.with_added(LineAxis::Vertical).is_err());
        assert!(full.with_added(LineAxis::Horizontal).is_ok());
    }

    #[test]
    fn a_line_can_be_added_at_a_position_even_while_locked_but_not_beyond_the_limit() {
        let lines = GuideLines::default().with_locked(true);
        let added = lines.with_added_at(LineAxis::Horizontal, -320).unwrap();
        assert_eq!(added.horizontal, vec![-320]);
        assert!(added.locked && added.shown);
        assert!(lines.with_added_at(LineAxis::Vertical, LINE_LIMIT).is_ok());
        assert!(
            lines
                .with_added_at(LineAxis::Vertical, -LINE_LIMIT - 1)
                .is_err()
        );
    }

    #[test]
    fn moving_replaces_one_position_and_refuses_locked_unknown_and_far_lines() {
        let lines = GuideLines::default()
            .with_added(LineAxis::Vertical)
            .unwrap()
            .with_added(LineAxis::Vertical)
            .unwrap();
        let moved = lines.with_moved(LineAxis::Vertical, 1, -250).unwrap();
        assert_eq!(moved.vertical, vec![0, -250]);
        assert!(lines.with_moved(LineAxis::Vertical, 2, 5).is_err());
        assert!(lines.with_moved(LineAxis::Horizontal, 0, 5).is_err());
        assert!(
            lines
                .with_moved(LineAxis::Vertical, 0, LINE_LIMIT + 1)
                .is_err()
        );
        assert!(lines.with_moved(LineAxis::Vertical, 0, LINE_LIMIT).is_ok());
        let locked = lines.with_locked(true);
        assert!(locked.with_moved(LineAxis::Vertical, 0, 5).is_err());
        assert!(
            locked
                .with_locked(false)
                .with_moved(LineAxis::Vertical, 0, 5)
                .is_ok()
        );
    }

    #[test]
    fn clearing_drops_every_line_and_keeps_only_the_rulers() {
        let lines = GuideLines::default()
            .with_added(LineAxis::Vertical)
            .unwrap()
            .with_added(LineAxis::Horizontal)
            .unwrap()
            .with_locked(true)
            .with_shown(false)
            .with_rulers(true);
        let cleared = lines.cleared();
        assert!(cleared.is_empty());
        assert!(cleared.rulers && cleared.shown && !cleared.locked);
    }

    #[test]
    fn stored_lines_refuse_unknown_fields_and_out_of_range_values() {
        let lines = GuideLines::default()
            .with_added(LineAxis::Vertical)
            .unwrap();
        let value = serde_json::to_value(&lines).unwrap();
        assert_eq!(serde_json::from_value::<GuideLines>(value).unwrap(), lines);
        assert!(
            serde_json::from_value::<GuideLines>(serde_json::json!({
                "rulers": true, "shown": true, "locked": false,
                "vertical": [], "horizontal": [], "extra": 1
            }))
            .is_err()
        );
        let far = GuideLines {
            vertical: vec![LINE_LIMIT + 1],
            ..Default::default()
        };
        assert!(far.validate().is_err());
        let many = GuideLines {
            horizontal: vec![0; LINES_MAX + 1],
            ..Default::default()
        };
        assert!(many.validate().is_err());
    }

    #[test]
    fn the_major_step_is_the_smallest_one_two_five_or_seven_that_leaves_few_spans() {
        let max = f64::from(kit::size::PLAYER_RULER_MAX_SPANS);
        for scale in [0.05_f32, 0.1, 0.1016, 0.25, 0.5, 1.0, 3.0, 16.0] {
            for length in [120.0_f32, 391.0, 442.0, 551.0, 900.0] {
                let step = major_step(scale, length).unwrap();
                let spans = f64::from(length) / f64::from(scale) / step;
                assert!(spans <= max + 1e-9, "{scale} {length}: {spans}");
                let mantissa = step / 10f64.powf(step.log10().floor());
                assert!(
                    [1.0, 2.0, 5.0, 7.0]
                        .iter()
                        .any(|m| (m - mantissa).abs() < 1e-9),
                    "{scale} {length}: {step}"
                );
                // No smaller allowed step would have fit.
                let smaller = match mantissa.round() as u32 {
                    1 => step * 0.7,
                    2 => step / 2.0,
                    5 => step * 2.0 / 5.0,
                    _ => step * 5.0 / 7.0,
                };
                assert!(
                    f64::from(length) / f64::from(scale) / smaller > max,
                    "{scale} {length}: {step}"
                );
            }
        }
        // The reference shots: top ruler 391 px logical at 0.1009 px per unit shows 700 (100216),
        // the left ruler 551 px at 0.102 shows 1000 (100216), and 442 and 383 px at 0.1018
        // show 700 (111304).
        assert_eq!(major_step(0.1009, 391.0), Some(700.0));
        assert_eq!(major_step(0.102, 551.0), Some(1000.0));
        assert_eq!(major_step(0.1018, 442.0), Some(700.0));
        assert_eq!(major_step(0.1018, 383.0), Some(700.0));
        assert_eq!(major_step(0.0, 400.0), None);
        assert_eq!(major_step(0.1, 0.0), None);
        assert_eq!(major_step(f32::NAN, 400.0), None);
    }

    #[test]
    fn marks_run_ten_to_a_major_step_and_put_zero_on_a_major_tick() {
        let marks = marks(-1500.0, 1500.0, 1000.0);
        assert_eq!(marks.first().unwrap().value, -1500.0);
        assert_eq!(marks.last().unwrap().value, 1500.0);
        let majors: Vec<f64> = marks
            .iter()
            .filter(|mark| mark.major)
            .map(|mark| mark.value)
            .collect();
        assert_eq!(majors, vec![-1000.0, 0.0, 1000.0]);
        assert_eq!(marks.len(), 31);
        assert!(super::marks(5.0, 1.0, 10.0).is_empty());
        assert!(super::marks(0.0, 10.0, 0.0).is_empty());
        assert_eq!(label(-0.0), "0");
        assert_eq!(label(-700.0), "-700");
    }

    #[test]
    fn screen_and_canvas_positions_invert_with_y_up_and_the_canvas_centre_at_zero() {
        let origin = egui::pos2(300.0, 200.0);
        assert_eq!(screen(origin, 0.5, LineAxis::Vertical, 100.0), 350.0);
        assert_eq!(screen(origin, 0.5, LineAxis::Horizontal, 100.0), 150.0);
        let pointer = egui::pos2(350.0, 150.0);
        assert_eq!(
            canvas_position(origin, 0.5, LineAxis::Vertical, pointer),
            100
        );
        assert_eq!(
            canvas_position(origin, 0.5, LineAxis::Horizontal, pointer),
            100
        );
        let far = egui::pos2(1.0e9, -1.0e9);
        assert_eq!(
            canvas_position(origin, 0.5, LineAxis::Vertical, far),
            LINE_LIMIT
        );
        assert_eq!(
            canvas_position(origin, 0.5, LineAxis::Horizontal, far),
            LINE_LIMIT
        );
    }
}
