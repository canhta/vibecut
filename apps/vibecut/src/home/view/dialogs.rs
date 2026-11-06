//! Draws the rename, info and delete dialogs over the Home page.

use super::widgets::{action_button, display_time, set_author_id};
use crate::{
    catalog::{self, Preferences, Text},
    egui,
    home::state::State,
    kit::{self, Icon},
};
use project::ProjectId;

/// Modal flows opened from a project card; Escape closes them while idle.
pub(super) fn show(state: &mut State, ctx: &egui::Context, preferences: &Preferences) {
    let locale = preferences.locale;
    let dark = preferences.dark;
    let busy = state.busy();
    let escape = ctx.input(|input| input.key_pressed(egui::Key::Escape));
    if escape && !busy {
        state.rename = None;
        state.info = None;
        state.delete = None;
    }

    let mut rename_submit: Option<(ProjectId, String)> = None;
    if let Some(dialog) = &mut state.rename {
        let mut save = false;
        let mut cancel = false;
        let title = catalog::text(locale, Text::ProjectsRenameTitle);
        let response =
            egui::Modal::new(egui::Id::unique("projects.rename.dialog")).show(ctx, |ui| {
                ui.heading(title);
                let response =
                    ui.add(egui::TextEdit::singleline(&mut dialog.name).desired_width(320.0));
                set_author_id(&response, "projects.rename.input");
                if let Some(error) = &dialog.error {
                    ui.label(
                        egui::RichText::new(catalog::text(locale, Text::ProjectsRenameInvalid))
                            .color(kit::error_text(dark)),
                    );
                    if !error.is_empty() {
                        ui.label(
                            egui::RichText::new(error)
                                .small()
                                .color(kit::secondary(dark)),
                        );
                    }
                }
                ui.horizontal(|ui| {
                    if action_button(
                        ui,
                        "projects.rename.save",
                        Icon::FloppyDisk,
                        catalog::text(locale, Text::ProjectsRenameSave),
                        !busy,
                    ) {
                        save = true;
                    }
                    if action_button(
                        ui,
                        "projects.rename.cancel",
                        Icon::X,
                        catalog::text(locale, Text::Cancel),
                        !busy,
                    ) {
                        cancel = true;
                    }
                });
            });
        set_author_id(&response.response, "projects.rename.dialog");
        if save {
            let name = dialog.name.trim().to_owned();
            if name.is_empty() {
                dialog.error = Some(String::new());
            } else {
                dialog.error = None;
                rename_submit = Some((dialog.id, name));
            }
        }
        if cancel {
            state.rename = None;
        }
    }
    if let Some((id, name)) = rename_submit {
        state.request_rename(id, name);
    }

    if let Some(id) = state.info {
        let Some(entry) = state.entries.iter().find(|entry| entry.id == id) else {
            state.info = None;
            return;
        };
        let name = entry.name.as_str().to_owned();
        let duration = display_time(entry.duration);
        let created = catalog::date(locale, entry.created_unix_ms);
        let modified = catalog::date(locale, entry.modified_unix_ms);
        let title = catalog::text(locale, Text::ProjectsInfoTitle);
        let mut close = false;
        let response = egui::Modal::new(egui::Id::unique("projects.info.dialog")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading(title);
            ui.label(egui::RichText::new(name).strong().size(16.0));
            ui.label(catalog::format(
                locale,
                Text::ProjectsDuration,
                &[("duration", &duration)],
            ));
            ui.label(catalog::format(
                locale,
                Text::CreatedDate,
                &[("date", &created)],
            ));
            ui.label(catalog::format(
                locale,
                Text::ModifiedDate,
                &[("date", &modified)],
            ));
            if action_button(
                ui,
                "projects.info.close",
                Icon::X,
                catalog::text(locale, Text::ProjectsInfoClose),
                true,
            ) {
                close = true;
            }
        });
        set_author_id(&response.response, "projects.info.dialog");
        if close {
            state.info = None;
        }
    }

    if let Some(dialog) = &state.delete {
        let names: Vec<String> = dialog
            .ids
            .iter()
            .filter_map(|id| {
                state
                    .entries
                    .iter()
                    .find(|entry| entry.id == *id)
                    .map(|entry| entry.name.as_str().to_owned())
            })
            .collect();
        let mut confirm = false;
        let mut cancel = false;
        let title = catalog::text(locale, Text::ProjectsDeleteTitle);
        let response =
            egui::Modal::new(egui::Id::unique("projects.delete.dialog")).show(ctx, |ui| {
                ui.set_width(420.0);
                ui.heading(title);
                ui.label(catalog::text(locale, Text::ProjectsDeletePrompt));
                let names_label = names.join(", ");
                let response =
                    ui.add(egui::Label::new(egui::RichText::new(names_label).strong()).wrap());
                set_author_id(&response, "projects.delete.names");
                if let Some(error) = &dialog.error {
                    ui.label(egui::RichText::new(error).color(kit::error_text(dark)));
                }
                ui.horizontal(|ui| {
                    if action_button(
                        ui,
                        "projects.delete.confirm",
                        Icon::Trash,
                        catalog::text(locale, Text::ProjectsDeleteConfirm),
                        !busy,
                    ) {
                        confirm = true;
                    }
                    if action_button(
                        ui,
                        "projects.delete.cancel",
                        Icon::X,
                        catalog::text(locale, Text::Cancel),
                        !busy,
                    ) {
                        cancel = true;
                    }
                });
            });
        set_author_id(&response.response, "projects.delete.dialog");
        if confirm {
            let ids = dialog.ids.clone();
            state.request_delete(ids);
        }
        if cancel {
            state.delete = None;
        }
    }
}
