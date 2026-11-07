//! Routes the font popover's widgets and requested catalog actions.
use super::Panel;
use crate::editor::properties::{Action, egui};

impl Panel<'_, '_> {
    pub(super) fn font_dialog(&mut self, ctx: &egui::Context) {
        if !self.state.font_picker.open {
            return;
        }
        let anchor = self.state.font_picker.anchor.unwrap_or_else(|| {
            self.state.area.map_or_else(
                || egui::pos2(ctx.content_rect().right() - 360.0, 120.0),
                |area| egui::pos2(area.left() - 8.0, area.top() + 52.0),
            )
        });
        let event = crate::font_picker::show(
            &mut self.state.font_picker,
            ctx,
            self.cx.locale,
            self.cx.dark,
            &self.cx.preferences.font_picker,
            anchor,
        );
        for (id, response) in event.widgets {
            self.remember(id, &response);
        }
        for action in event.actions {
            self.actions.push(Action::Font(action));
        }
    }
}
