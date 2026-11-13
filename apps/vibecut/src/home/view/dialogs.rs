//! Draws the rename, info and delete dialogs over the Home page.

use super::widgets::{action_button, display_time, set_author_id};
use crate::{
    egui,
    home::state::State,
    i18n::{self, Text},
    kit::{self, Icon, Look},
};
use project::ProjectId;

/// Modal flows opened from a project card; Escape closes them while idle.
pub(super) fn show(state: &mut State, ctx: &egui::Context, look: Look) {
    let locale = look.locale;
    let dark = look.dark;
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
        let title = i18n::text(locale, Text::ProjectsRenameTitle);
        let response =
            egui::Modal::new(egui::Id::unique("projects.rename.dialog")).show(ctx, |ui| {
                ui.heading(title);
                let response =
                    ui.add(egui::TextEdit::singleline(&mut dialog.name).desired_width(320.0));
                set_author_id(&response, "projects.rename.input");
                if let Some(error) = &dialog.error {
                    ui.label(
                        egui::RichText::new(i18n::text(locale, Text::ProjectsRenameInvalid))
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
                        i18n::text(locale, Text::ProjectsRenameSave),
                        !busy,
                    ) {
                        save = true;
                    }
                    if action_button(
                        ui,
                        "projects.rename.cancel",
                        Icon::X,
                        i18n::text(locale, Text::Cancel),
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
        let created = i18n::date(locale, entry.created_unix_ms);
        let modified = i18n::date(locale, entry.modified_unix_ms);
        let title = i18n::text(locale, Text::ProjectsInfoTitle);
        let mut close = false;
        let response = egui::Modal::new(egui::Id::unique("projects.info.dialog")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading(title);
            ui.label(egui::RichText::new(name).strong().size(16.0));
            ui.label(i18n::format(
                locale,
                Text::ProjectsDuration,
                &[("duration", &duration)],
            ));
            ui.label(i18n::format(
                locale,
                Text::CreatedDate,
                &[("date", &created)],
            ));
            ui.label(i18n::format(
                locale,
                Text::ModifiedDate,
                &[("date", &modified)],
            ));
            if action_button(
                ui,
                "projects.info.close",
                Icon::X,
                i18n::text(locale, Text::ProjectsInfoClose),
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
        let permanent = dialog.permanent;
        let names: Vec<String> = dialog
            .ids
            .iter()
            .filter_map(|id| {
                state
                    .entries
                    .iter()
                    .chain(state.trash.iter())
                    .find(|entry| entry.id == *id)
                    .map(|entry| entry.name.as_str().to_owned())
            })
            .collect();
        let mut confirm = false;
        let mut cancel = false;
        let title = i18n::text(
            locale,
            if permanent {
                Text::ProjectsPurgeTitle
            } else {
                Text::ProjectsDeleteTitle
            },
        );
        let prompt = if permanent {
            i18n::text(locale, Text::ProjectsPurgePrompt).to_owned()
        } else {
            i18n::format(
                locale,
                Text::ProjectsDeletePrompt,
                &[(
                    "days",
                    &i18n::number(locale, engine::library::RETENTION_DAYS),
                )],
            )
        };
        let dialog_id = if permanent {
            "projects.purge.dialog"
        } else {
            "projects.delete.dialog"
        };
        let response = egui::Modal::new(egui::Id::unique(dialog_id)).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading(title);
            ui.label(prompt);
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
                    i18n::text(
                        locale,
                        if permanent {
                            Text::ProjectsPurgeConfirm
                        } else {
                            Text::ProjectsDeleteConfirm
                        },
                    ),
                    !busy,
                ) {
                    confirm = true;
                }
                if action_button(
                    ui,
                    "projects.delete.cancel",
                    Icon::X,
                    i18n::text(locale, Text::Cancel),
                    !busy,
                ) {
                    cancel = true;
                }
            });
        });
        set_author_id(&response.response, dialog_id);
        if confirm {
            let ids = dialog.ids.clone();
            if permanent {
                state.request_purge(ids);
            } else {
                state.request_delete(ids);
            }
        }
        if cancel {
            state.delete = None;
        }
    }
}
