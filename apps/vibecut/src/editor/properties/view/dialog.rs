//! Renders the colour window.
use super::Panel;
use crate::editor::properties::{Action, egui};

impl Panel<'_, '_> {
    /// Draw the open picker and turn its events into previews and commits.
    pub(super) fn picker_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut picker) = self.state.picker.take() else {
            return;
        };
        let event = picker.ui(ctx, self.cx.locale, &self.cx.preferences.recent_colors);
        for (id, response) in event.widgets {
            self.remember(id, &response);
        }
        if event.cancel {
            self.state.color_base = None;
            self.actions.push(Action::SyncPreview);
            return;
        }
        match picker
            .target
            .command(self.cx.session.project(), picker.color)
        {
            Ok(command) => {
                if event.commit && picker.color != picker.original {
                    let mut recent = self.cx.preferences.recent_colors.clone();
                    recent.retain(|c| *c != picker.color);
                    recent.insert(0, picker.color);
                    recent.truncate(16);
                    self.actions.push(Action::ColorCommit { command, recent });
                } else if event.preview {
                    self.actions.push(Action::ColorPreview { command });
                }
            }
            Err(error) => {
                self.fail(error);
                self.actions.push(Action::SyncPreview);
            }
        }
        self.state.picker = Some(picker);
    }
}
