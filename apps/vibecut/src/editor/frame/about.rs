//! Hosts the shared About dialog in the editor window and routes its clicks.
use crate::editor::Editor;
use crate::egui;
use crate::services::about;

impl Editor {
    pub(in crate::editor) fn show_about_dialog(&mut self, ctx: &egui::Context) {
        let Some(page) = self.frame.about else {
            return;
        };
        let painted = about::show(ctx, page, self.preferences.locale, self.preferences.dark);
        for (id, icon, response) in painted {
            self.remember_with_icon(id, icon, &response);
            if response.clicked() {
                self.action(id);
            }
        }
    }
}
