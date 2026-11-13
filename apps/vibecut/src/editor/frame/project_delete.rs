//! Presents the header Delete confirmation and routes its two choices.
use crate::editor::Editor;
use crate::i18n::{self, Text};
use crate::{egui, kit::Icon};

impl Editor {
    pub(in crate::editor) fn show_project_delete_dialog(&mut self, ctx: &egui::Context) {
        if !self.frame.delete_open {
            return;
        }
        let locale = self.preferences.locale;
        let mut painted = Vec::new();
        egui::Modal::new(egui::Id::unique("project.delete_dialog")).show(ctx, |ui| {
            ui.set_max_width(380.0);
            ui.heading(i18n::text(locale, Text::ProjectsDeleteTitle));
            ui.label(i18n::format(
                locale,
                Text::ProjectsDeletePrompt,
                &[(
                    "days",
                    &i18n::number(locale, engine::library::RETENTION_DAYS),
                )],
            ));
            ui.horizontal(|ui| {
                for (id, icon, text) in [
                    ("project.cancel_delete", None, Text::Cancel),
                    (
                        "project.confirm_delete",
                        Some(Icon::Trash),
                        Text::ProjectsDeleteConfirm,
                    ),
                ] {
                    let response =
                        ui.add_sized([120.0, 34.0], egui::Button::new(i18n::text(locale, text)));
                    painted.push((id, icon.map(|icon| icon.name()), response));
                }
            });
        });
        for (id, icon, response) in painted {
            self.remember_with_icon(id, icon, &response);
            if response.clicked() {
                self.action(id);
            }
        }
    }
}
