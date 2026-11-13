//! Renders the export dialog into the frame requests.
use super::{Action, State, dialog::DialogAction};
use crate::editor::{Context, Text, egui};

pub(super) fn show(state: &mut State, cx: &Context<'_>, ctx: &egui::Context) -> Vec<Action> {
    let mut actions = Vec::new();
    let project = cx.session.project();
    let preferred = cx.preferences.export;
    let Some(dialog) = state.dialog_mut() else {
        return actions;
    };
    let items: Vec<_> = project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .filter(|item| cx.selection().contains(&item.id))
        .collect();
    dialog.selection_range = items
        .iter()
        .map(|item| item.start)
        .min()
        .zip(items.iter().filter_map(|item| item.end().ok()).max())
        .and_then(|(start, end)| time::TimeRange::new(start, end).ok());
    let mut options = preferred;
    let output = dialog.ui(ctx, cx.locale, cx.dark, &mut options, project, cx.preview);
    match output.action {
        Some(DialogAction::Start) => actions.push(Action::Start(options)),
        Some(DialogAction::Retry) => actions.push(Action::Retry),
        Some(DialogAction::Save) => actions.push(Action::Save),
        None => {}
    }
    if dialog.take_save_request(project, preferred).is_some() {
        actions.push(Action::Save);
    }
    if let Some(saved) = dialog.take_saved() {
        actions.push(Action::Saved(saved));
    }
    if options != preferred {
        actions.push(Action::Options(options));
    }
    for (id, response) in output.widgets {
        actions.push(Action::Remember { id, response });
    }
    actions
}

/// Apply one request from the dialog; a failure stays inside the dialog.
pub(super) fn apply(desk: &mut crate::editor::Desk<'_>, action: Action) {
    let editor = desk.editor();
    match action {
        Action::Start(options) => {
            let blocked = editor.session.transaction_active();
            let project = editor.session.project().clone();
            if let Err(error) = editor.dialogs.export.start(project, options, blocked) {
                editor.dialogs.export.fail(error);
            }
        }
        Action::Retry => {
            let blocked = editor.session.transaction_active();
            let project = editor.session.project().clone();
            if let Err(error) = editor.dialogs.export.retry(project, blocked) {
                editor.dialogs.export.fail(error);
            }
        }
        Action::Save => {
            editor.lifecycle_action = Some(crate::lifecycle::EditorAction::ExportSave);
        }
        Action::Options(options) => {
            editor
                .desk()
                .set_preference("export", serde_json::json!(options));
        }
        Action::Saved(saved) => {
            editor.preferences.export_directory = saved.parent().map(|path| path.to_owned());
            let text = editor.tr(Text::ExportSaved);
            editor.set_toast(text, Some(saved.to_string_lossy().into_owned()));
        }
        Action::Remember { id, response } => editor.remember(id, &response),
    }
}
