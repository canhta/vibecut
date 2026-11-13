//! Renders asset deletion confirmation.
use super::super::{Action, Id, Recorded, State, Text, egui};
use crate::editor::Context;
use crate::i18n;

pub(in crate::editor) fn show(state: &State, cx: &Context<'_>, ctx: &egui::Context) -> Vec<Action> {
    let Some(ids) = &state.media_delete_confirm else {
        return Vec::new();
    };
    let mut actions = Vec::new();
    egui::Modal::new(Id::unique("media.delete_dialog"))
        .frame(crate::kit::dialog_frame(ctx))
        .show(ctx, |ui| {
            ui.set_max_width(380.0);
            crate::kit::dialog_title(
                ui,
                &format!("{} {}", cx.tr(Text::Delete), cx.tr(Text::Media)),
            );
            ui.label(i18n::format_count(
                cx.locale,
                ids.len() as u64,
                Text::DeleteMediaPromptOne,
                Text::DeleteMediaPromptOther,
            ));
            ui.horizontal(|ui| {
                let response = crate::kit::dialog_button(
                    ui,
                    "media.cancel_delete",
                    cx.tr(Text::Cancel),
                    false,
                );
                remember(&mut actions, "media.cancel_delete", &response);
                if response.clicked() {
                    actions.push(Action::Command("media.cancel_delete".into()));
                }
                let response = crate::kit::dialog_button(
                    ui,
                    "media.confirm_delete",
                    cx.tr(Text::Delete),
                    false,
                );
                remember(&mut actions, "media.confirm_delete", &response);
                if response.clicked() {
                    actions.push(Action::Command("media.confirm_delete".into()));
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
