//! The collapsed timeline's progress line.
//!
//! One rail under the toolbar, the width of the panel less the toolbar's edge inset, with the
//! playhead as its handle. It reuses the zoom rail's measured parts (2 px rail, 8×12 handle).
//! The strip has the height of the foot strip under the lanes. Its place and height are ours:
//! the web capture shows the line under the toolbar but no desktop capture measures it.

use super::toolbar::EDGE;
use crate::editor::timeline::{
    Action, Context, Rect, Rounding, State, Tick, TimelineLayout, egui, frame_snap,
};

impl State {
    /// Paint the rail; a click or drag on it seeks to that fraction of the project.
    pub(in crate::editor::timeline) fn progress_line(
        &mut self,
        cx: &Context<'_>,
        ui: &mut egui::Ui,
        layout: &TimelineLayout,
    ) {
        let panel = ui.max_rect();
        let strip = Rect::from_min_max(
            egui::pos2(
                panel.left() + EDGE,
                (layout.toolbar.bottom() + 1.0).min(panel.bottom()),
            ),
            egui::pos2(panel.right() - EDGE, panel.bottom()),
        );
        if strip.width() <= 0.0 || strip.height() <= 0.0 {
            return;
        }
        let duration = cx.staged_project().duration();
        let total = duration.seconds();
        let mut value = if total > 0.0 {
            (cx.playhead.seconds() / total).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let response = crate::kit::zoom_rail(ui, "timeline.progress", strip, &mut value);
        if response.changed() && total > 0.0 {
            let at = Tick::from_seconds_f64(f64::from(value) * total, Rounding::Nearest)
                .unwrap_or(Tick::ZERO)
                .clamp(Tick::ZERO, duration);
            self.push(Action::Seek(frame_snap(cx, at)));
        }
        self.remember("timeline.progress", &response);
    }
}
