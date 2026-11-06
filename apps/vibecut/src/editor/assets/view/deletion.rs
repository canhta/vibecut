//! Renders asset deletion confirmation.
use crate::editor::assets::{Editor, Id, Text, catalog, egui};

impl Editor {
    pub(in crate::editor) fn media_delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(ids) = self.media_delete_confirm.clone() else {
            return;
        };
        egui::Modal::new(Id::unique("media.delete_dialog")).show(ctx, |ui| {
            ui.set_max_width(380.0);
            ui.heading(format!(
                "{} {}",
                self.tr(Text::Delete),
                self.tr(Text::Media)
            ));
            ui.label(catalog::format_count(
                self.preferences.locale,
                ids.len() as u64,
                Text::DeleteMediaPromptOne,
                Text::DeleteMediaPromptOther,
            ));
            ui.horizontal(|ui| {
                let response = ui.add_sized([90.0, 34.0], egui::Button::new(self.tr(Text::Cancel)));
                self.remember("media.cancel_delete", &response);
                if response.clicked() {
                    self.action("media.cancel_delete");
                }
                let response = ui.add_sized([90.0, 34.0], egui::Button::new(self.tr(Text::Delete)));
                self.remember("media.confirm_delete", &response);
                if response.clicked() {
                    self.action("media.confirm_delete");
                }
            });
        });
    }
}
