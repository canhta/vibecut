//! Renders the export dialog into the frame requests.
use super::{Action, State, dialog::DialogAction};
use crate::editor::{Context, Port, Request, Text, egui};
use crate::services::lifecycle::EditorAction;

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
        Some(DialogAction::Caption(output)) => actions.push(Action::Caption(output)),
        None => {}
    }
    if dialog.take_save_request(project, preferred).is_some() {
        actions.push(Action::Save);
    }
    for (path, output) in dialog.take_caption_writes() {
        actions.push(Action::CaptionFile { path, output });
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
pub(super) fn apply(state: &mut State, port: &mut Port<'_>, action: Action) {
    match action {
        Action::Start(options) => {
            let blocked = port.session.transaction_active();
            let encoder = port
                .read
                .preferences
                .hardware_acceleration
                .encode_preference();
            let project = port.session.project().clone();
            if let Err(error) = state.start(project, options, blocked, encoder) {
                state.fail(error);
            }
        }
        Action::Retry => {
            let blocked = port.session.transaction_active();
            let project = port.session.project().clone();
            if let Err(error) = state.retry(project, blocked) {
                state.fail(error);
            }
        }
        Action::Save => port.request(Request::Lifecycle(EditorAction::ExportSave)),
        Action::CaptionFile { path, output } => {
            port.request(Request::CaptionFile { path, output });
        }
        Action::Caption(output) => {
            port.request(Request::Lifecycle(EditorAction::ExportCaption(output)));
        }
        Action::Options(options) => {
            port.set_preference("export", serde_json::json!(options));
        }
        Action::Saved(saved) => {
            let mut preferences = port.read.preferences.clone();
            preferences.export_directory = saved
                .last()
                .and_then(|path| path.parent())
                .map(|path| path.to_owned());
            port.adopt_preferences(preferences);
            let text = port.tr(Text::ExportSaved);
            // One toast names every file the export wrote, one path per line.
            let files: Vec<_> = saved
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect();
            port.toast(text, Some(files.join("\n")));
        }
        Action::Remember { id, response } => port.widget(id, None, None, &response),
    }
}
