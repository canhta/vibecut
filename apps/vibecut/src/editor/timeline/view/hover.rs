//! Tracks the preview-axis hover time and paints its guide.
//!
//! The hover time is presentation state only: it never reaches the project or
//! the transport. The frame asks the playback service to render at it.

use crate::editor::timeline::{
    Context, Rect, State, Tick, TimelineLayout, Tool, egui, frame_snap, view::Ink,
};

impl State {
    /// The Split-mode cut time under the pointer over the body: frame-snapped and
    /// never before the origin. `None` outside Split mode or off the body.
    pub(in crate::editor::timeline) fn split_cut(
        &self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        layout: &TimelineLayout,
    ) -> Option<Tick> {
        if self.tool != Tool::Split {
            return None;
        }
        ui.ctx()
            .input(|input| input.pointer.hover_pos())
            .filter(|pos| layout.body.contains(*pos))
            .map(|pos| frame_snap(cx, self.tick_at(pos.x, layout.body.left())).max(Tick::ZERO))
    }

    /// Split mode: the crosshair cursor over the body and a guide at the cut time.
    pub(in crate::editor::timeline) fn paint_split_cut(
        &self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        layout: &TimelineLayout,
    ) {
        let Some(at) = self.split_cut(cx, ui, layout) else {
            return;
        };
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        let x = self.x_at(at, layout.body.left());
        if x < layout.body.left() - 1.0 || x > layout.body.right() + 1.0 {
            return;
        }
        let painter = ui.painter().with_clip_rect(Rect::from_min_max(
            layout.ruler.min,
            egui::pos2(layout.body.right(), layout.body.bottom()),
        ));
        let x = x.round() + 0.5;
        painter.line_segment(
            [
                egui::pos2(x, layout.ruler.top()),
                egui::pos2(x, layout.body.bottom()),
            ],
            egui::Stroke::new(1.0, Ink::of(cx).text()),
        );
    }

    /// Sample the pointer time over the ruler or lanes; clear it on leave or
    /// when the axis is off. Empty tail remains a pointer time, not a seek.
    pub(in crate::editor::timeline) fn track_hover(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        layout: &TimelineLayout,
    ) {
        self.hover = if cx.preferences.preview_axis {
            ui.ctx()
                .input(|input| input.pointer.hover_pos())
                .filter(|pos| layout.ruler.contains(*pos) || layout.body.contains(*pos))
                .map(|pos| frame_snap(cx, self.tick_at(pos.x, layout.body.left())).max(Tick::ZERO))
        } else {
            None
        };
    }

    /// The vertical guide at the hover time, over the ruler and every lane.
    pub(in crate::editor::timeline) fn paint_hover(
        &self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        layout: &TimelineLayout,
    ) {
        let Some(at) = self.hover else {
            return;
        };
        let x = self.x_at(at, layout.body.left());
        if x < layout.ruler.left() - 1.0 || x > layout.ruler.right() + 1.0 {
            return;
        }
        let painter = ui.painter().with_clip_rect(Rect::from_min_max(
            layout.ruler.min,
            egui::pos2(layout.ruler.right(), layout.body.bottom()),
        ));
        let x = x.round() + 0.5;
        painter.line_segment(
            [
                egui::pos2(x, layout.ruler.top()),
                egui::pos2(x, layout.body.bottom()),
            ],
            egui::Stroke::new(1.0, Ink::of(cx).text().gamma_multiply(0.7)),
        );
    }
}
