//! Renders scene selection.
use crate::editor::{BUTTON_ICON, Editor, Icon, Id, Text, Vec2, egui, icons, theme};

pub(in crate::editor) fn show_scenes(editor: &mut Editor, ctx: &egui::Context) {
    if !editor.scenes_open {
        return;
    }
    let scenes: Vec<_> = editor
        .session
        .project()
        .data()
        .scenes
        .iter()
        .map(|s| (s.id, s.name.as_str().to_owned(), s.is_main))
        .collect();
    let active = editor.session.project().data().current_scene_id;
    let sheet_id = Id::unique("scenes.sheet");
    let sheet = egui::Modal::new(sheet_id)
        .area(egui::Modal::default_area(sheet_id).anchor(egui::Align2::RIGHT_TOP, Vec2::ZERO))
        .show(ctx, |ui| {
            ui.set_width(320.0);
            ui.set_min_height((ctx.content_rect().height() - 16.0).max(200.0));
            ui.heading(editor.tr(if editor.scene_select {
                Text::SelectScenes
            } else {
                Text::Scenes
            }));
            ui.label(editor.tr(if editor.scene_select {
                Text::SelectScenesHint
            } else {
                Text::SwitchScenesHint
            }));
            ui.horizontal(|ui| {
                let response = ui
                    .add(
                        egui::Button::new(icons::tinted(
                            Icon::Plus,
                            BUTTON_ICON,
                            editor.text_color(),
                        ))
                        .min_size(Vec2::splat(28.0)),
                    )
                    .on_hover_text(editor.tr(Text::NewScene));
                editor.remember_with_icon("scene.create", Some(Icon::Plus.name()), &response);
                if response.clicked() {
                    editor.action("scene.create");
                }
                let response = ui.button(editor.tr(if editor.scene_select {
                    Text::CancelSelection
                } else {
                    Text::Select
                }));
                editor.remember("scene.select", &response);
                if response.clicked() {
                    editor.action("scene.select");
                }
                let response = ui.button(editor.tr(Text::CloseScenes));
                editor.remember("scene.close", &response);
                if response.clicked() {
                    editor.action("scene.close");
                }
            });
            let editing = editor.scene_rename.as_ref().map(|(id, _)| *id);
            egui::ScrollArea::vertical()
                .max_height(420.0)
                .show(ui, |ui| {
                    for (id, name, main) in scenes {
                        if editing == Some(id) {
                            let mut text = editor
                                .scene_rename
                                .as_ref()
                                .map(|(_, text)| text.clone())
                                .unwrap_or_default();
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut text)
                                    .id_salt("scene.rename.input")
                                    .desired_width(296.0)
                                    .min_size(Vec2::new(296.0, 26.0)),
                            );
                            editor.remember("scene.rename.input", &response);
                            if editor.scene_rename_focus {
                                response.request_focus();
                                editor.scene_rename_focus = false;
                            }
                            let cancel = !editor.scene_confirm
                                && ui.input(|i| i.key_pressed(egui::Key::Escape));
                            let commit = !editor.scene_confirm
                                && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if commit && !cancel {
                                match editor.session.execute(
                                    "scene.rename",
                                    serde_json::json!({"scene":id,"name":text}),
                                ) {
                                    Ok(_) => editor.scene_rename = None,
                                    Err(error) => {
                                        editor.error = Some(error.to_string());
                                        editor.scene_rename = Some((id, text));
                                    }
                                }
                            } else if cancel {
                                editor.scene_rename = None;
                            } else {
                                editor.scene_rename = Some((id, text));
                            }
                            continue;
                        }
                        let selected = if editor.scene_select {
                            editor.scene_selected.contains(&id)
                        } else {
                            active == id
                        };
                        let label = if main {
                            format!("{name} ({})", editor.tr(Text::MainScene))
                        } else {
                            name
                        };
                        let response = ui.add_enabled(
                            !editor.scene_select || !main,
                            egui::Button::new(egui::RichText::new(label).color(if selected {
                                theme::on_accent(editor.preferences.dark)
                            } else {
                                editor.text_color()
                            }))
                            .truncate()
                            .selected(selected)
                            .min_size(Vec2::new(296.0, 40.0)),
                        );
                        let rename_id = format!("scene.rename.{}", id.0);
                        let mut request_rename = false;
                        let mut menu_item = None;
                        response.context_menu(|ui| {
                            let item = ui.add(egui::Button::image_and_text(
                                icons::tinted(Icon::PencilSimple, BUTTON_ICON, editor.text_color()),
                                editor.tr(Text::RenameScene),
                            ));
                            if item.clicked() {
                                request_rename = true;
                                ui.close();
                            }
                            menu_item = Some(item);
                        });
                        if let Some(item) = menu_item {
                            editor.remember_with_icon(
                                &rename_id,
                                Some(Icon::PencilSimple.name()),
                                &item,
                            );
                        }
                        if request_rename {
                            editor.action(&rename_id);
                        }
                        let row = format!("scene.row.{}", id.0);
                        editor.remember(&row, &response);
                        if response.clicked() {
                            editor.action(&row);
                        }
                    }
                });
            if editor.scene_select {
                let response = ui.add_enabled(
                    !editor.scene_selected.is_empty(),
                    egui::Button::image_and_text(
                        icons::tinted(Icon::Trash, BUTTON_ICON, editor.text_color()),
                        format!(
                            "{} ({})",
                            editor.tr(Text::Delete),
                            editor.scene_selected.len()
                        ),
                    ),
                );
                editor.remember("scene.delete", &response);
                if response.clicked() {
                    editor.action("scene.delete");
                }
            }
        });
    if sheet.backdrop_response.clicked() && !editor.scene_confirm && editor.scene_rename.is_none() {
        editor.action("scene.close");
    }
    if editor.scene_confirm {
        egui::Modal::new(Id::unique("scene.delete_dialog")).show(ctx, |ui| {
            ui.set_max_width(360.0);
            ui.heading(format!(
                "{} {}",
                editor.tr(Text::Delete),
                editor.tr(Text::Scenes)
            ));
            ui.label(editor.tr(Text::DeleteScenesPrompt));
            ui.horizontal(|ui| {
                let response =
                    ui.add_sized([90.0, 34.0], egui::Button::new(editor.tr(Text::Cancel)));
                editor.remember("scene.cancel_delete", &response);
                if response.clicked() {
                    editor.action("scene.cancel_delete");
                }
                let response =
                    ui.add_sized([90.0, 34.0], egui::Button::new(editor.tr(Text::Delete)));
                editor.remember("scene.confirm_delete", &response);
                if response.clicked() {
                    editor.action("scene.confirm_delete");
                }
            });
        });
    }
}
