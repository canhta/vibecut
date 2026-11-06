//! Renders scene selection.
use crate::editor::timeline::{
    Action, BUTTON_ICON, Context, Icon, Id, State, Text, Vec2, egui, icons, theme,
    view::{Ink, tr},
};

/// Paint the scene sheet and its delete confirmation when open.
pub(in crate::editor) fn show_scenes(
    state: &mut State,
    cx: &Context<'_>,
    ctx: &egui::Context,
) -> Vec<Action> {
    if !state.scenes.open {
        return Vec::new();
    }
    let ink = Ink::of(cx);
    let scenes: Vec<_> = cx
        .session
        .project()
        .data()
        .scenes
        .iter()
        .map(|s| (s.id, s.name.as_str().to_owned(), s.is_main))
        .collect();
    let active = cx.session.project().data().current_scene_id;
    let sheet_id = Id::unique("scenes.sheet");
    let sheet = egui::Modal::new(sheet_id)
        .area(egui::Modal::default_area(sheet_id).anchor(egui::Align2::RIGHT_TOP, Vec2::ZERO))
        .show(ctx, |ui| {
            ui.set_width(320.0);
            ui.set_min_height((ctx.content_rect().height() - 16.0).max(200.0));
            let selecting = state.scenes.select;
            ui.heading(tr(
                cx,
                if selecting {
                    Text::SelectScenes
                } else {
                    Text::Scenes
                },
            ));
            ui.label(tr(
                cx,
                if selecting {
                    Text::SelectScenesHint
                } else {
                    Text::SwitchScenesHint
                },
            ));
            ui.horizontal(|ui| {
                let response = ui
                    .add(
                        egui::Button::new(icons::tinted(Icon::Plus, BUTTON_ICON, ink.text()))
                            .min_size(Vec2::splat(28.0)),
                    )
                    .on_hover_text(tr(cx, Text::NewScene));
                state.remember_icon("scene.create", Some(Icon::Plus.name()), &response);
                if response.clicked() {
                    state.push(Action::Activate("scene.create".to_owned()));
                }
                let response = ui.button(tr(
                    cx,
                    if selecting {
                        Text::CancelSelection
                    } else {
                        Text::Select
                    },
                ));
                state.remember("scene.select", &response);
                if response.clicked() {
                    state.push(Action::Activate("scene.select".to_owned()));
                }
                let response = ui.button(tr(cx, Text::CloseScenes));
                state.remember("scene.close", &response);
                if response.clicked() {
                    state.push(Action::Activate("scene.close".to_owned()));
                }
            });
            let editing = state.scenes.rename.as_ref().map(|(id, _)| *id);
            egui::ScrollArea::vertical()
                .max_height(420.0)
                .show(ui, |ui| {
                    for (id, name, main) in scenes {
                        if editing == Some(id) {
                            let mut text = state
                                .scenes
                                .rename
                                .as_ref()
                                .map(|(_, text)| text.clone())
                                .unwrap_or_default();
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut text)
                                    .id_salt("scene.rename.input")
                                    .desired_width(296.0)
                                    .min_size(Vec2::new(296.0, 26.0)),
                            );
                            state.remember("scene.rename.input", &response);
                            if state.scenes.rename_focus {
                                response.request_focus();
                                state.scenes.rename_focus = false;
                            }
                            let confirming = state.scenes.confirm;
                            let cancel =
                                !confirming && ui.input(|i| i.key_pressed(egui::Key::Escape));
                            let commit =
                                !confirming && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if commit && !cancel {
                                // The frame clears the field on success and gives the
                                // text back through `keep_scene_rename` on refusal.
                                state.scenes.rename = None;
                                state.push(Action::RenameScene {
                                    scene: id,
                                    name: text,
                                });
                            } else if cancel {
                                state.scenes.rename = None;
                            } else {
                                state.scenes.rename = Some((id, text));
                            }
                            continue;
                        }
                        let selected = if selecting {
                            state.scenes.selected.contains(&id)
                        } else {
                            active == id
                        };
                        let label = if main {
                            format!("{name} ({})", tr(cx, Text::MainScene))
                        } else {
                            name
                        };
                        let response = ui.add_enabled(
                            !selecting || !main,
                            egui::Button::new(egui::RichText::new(label).color(if selected {
                                theme::on_accent(ink.dark())
                            } else {
                                ink.text()
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
                                icons::tinted(Icon::PencilSimple, BUTTON_ICON, ink.text()),
                                tr(cx, Text::RenameScene),
                            ));
                            if item.clicked() {
                                request_rename = true;
                                ui.close();
                            }
                            menu_item = Some(item);
                        });
                        if let Some(item) = menu_item {
                            state.remember_icon(
                                rename_id.clone(),
                                Some(Icon::PencilSimple.name()),
                                &item,
                            );
                        }
                        if request_rename {
                            state.push(Action::Activate(rename_id));
                        }
                        let row = format!("scene.row.{}", id.0);
                        state.remember(row.clone(), &response);
                        if response.clicked() {
                            state.push(Action::Activate(row));
                        }
                    }
                });
            if selecting {
                let response = ui.add_enabled(
                    !state.scenes.selected.is_empty(),
                    egui::Button::image_and_text(
                        icons::tinted(Icon::Trash, BUTTON_ICON, ink.text()),
                        format!("{} ({})", tr(cx, Text::Delete), state.scenes.selected.len()),
                    ),
                );
                state.remember("scene.delete", &response);
                if response.clicked() {
                    state.push(Action::Activate("scene.delete".to_owned()));
                }
            }
        });
    if sheet.backdrop_response.clicked() && !state.scenes.confirm && state.scenes.rename.is_none() {
        state.push(Action::Activate("scene.close".to_owned()));
    }
    if state.scenes.confirm {
        egui::Modal::new(Id::unique("scene.delete_dialog")).show(ctx, |ui| {
            ui.set_max_width(360.0);
            ui.heading(format!("{} {}", tr(cx, Text::Delete), tr(cx, Text::Scenes)));
            ui.label(tr(cx, Text::DeleteScenesPrompt));
            ui.horizontal(|ui| {
                let response = ui.add_sized([90.0, 34.0], egui::Button::new(tr(cx, Text::Cancel)));
                state.remember("scene.cancel_delete", &response);
                if response.clicked() {
                    state.push(Action::Activate("scene.cancel_delete".to_owned()));
                }
                let response = ui.add_sized([90.0, 34.0], egui::Button::new(tr(cx, Text::Delete)));
                state.remember("scene.confirm_delete", &response);
                if response.clicked() {
                    state.push(Action::Activate("scene.confirm_delete".to_owned()));
                }
            });
        });
    }
    std::mem::take(&mut state.outbox)
}
