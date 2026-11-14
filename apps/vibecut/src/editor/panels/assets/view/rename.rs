//! Renders the media Rename dialog.
use super::super::{Action, Id, Recorded, State, Text, egui};
use crate::editor::Context;

pub(in crate::editor) fn show(
    state: &mut State,
    cx: &Context<'_>,
    ctx: &egui::Context,
) -> Vec<Action> {
    let Some(rename) = &mut state.media_rename else {
        return Vec::new();
    };
    let mut actions = Vec::new();
    egui::Modal::new(Id::unique("media.rename_dialog"))
        .frame(crate::kit::dialog_frame(ctx))
        .show(ctx, |ui| {
            ui.set_max_width(380.0);
            crate::kit::dialog_title(ui, cx.tr(Text::MediaRename));
            let field = ui.add(
                egui::TextEdit::singleline(&mut rename.draft)
                    .desired_width(f32::INFINITY)
                    .id(Id::unique("media.rename_field")),
            );
            if !field.has_focus() && !field.lost_focus() && !field.changed() {
                field.request_focus();
            }
            remember(&mut actions, "media.rename_field", &field);
            let valid = !rename.draft.trim().is_empty();
            let submitted =
                field.lost_focus() && ctx.input(|input| input.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                let response = crate::kit::dialog_button(
                    ui,
                    "media.rename_cancel",
                    cx.tr(Text::Cancel),
                    false,
                );
                remember(&mut actions, "media.rename_cancel", &response);
                if response.clicked() {
                    actions.push(Action::MediaRenameCancel);
                }
                let confirm = ui
                    .add_enabled_ui(valid, |ui| {
                        crate::kit::dialog_button(
                            ui,
                            "media.rename_confirm",
                            cx.tr(Text::MediaRename),
                            true,
                        )
                    })
                    .inner;
                remember(&mut actions, "media.rename_confirm", &confirm);
                if confirm.clicked() || (submitted && valid) {
                    actions.push(Action::MediaRenameConfirm);
                }
            });
        });
    actions
}

fn remember(actions: &mut Vec<Action>, id: &str, response: &egui::Response) {
    response
        .ctx
        .accesskit_node_builder(response.id, |node| node.set_author_id(id));
    actions.push(Action::Record(Box::new(Recorded {
        id: id.to_owned(),
        response: response.clone(),
        glyph: None,
        tooltip: None,
    })));
}
