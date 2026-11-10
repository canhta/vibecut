//! Routes the font popover's widgets and requested catalog actions.
use super::Panel;
use crate::editor::properties::{Action, egui};

impl Panel<'_, '_> {
    pub(super) fn font_dialog(&mut self, ctx: &egui::Context) {
        if !self.state.font_picker.open {
            return;
        }
        // The row stores the field rect; open below its left edge and let the
        // window's own constraint keep the popover inside the app frame.
        let anchor = self
            .state
            .font_picker
            .anchor
            .unwrap_or_else(|| egui::pos2(ctx.content_rect().left() + 24.0, 120.0));
        let event = crate::editor::properties::font_picker::show(
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
