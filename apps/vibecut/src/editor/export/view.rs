//! Renders the export window.
use crate::editor::{Editor, Error, PREFERENCE_COMMAND, Path, Text, egui};

pub(in crate::editor) fn show(editor: &mut Editor, ctx: &egui::Context) {
    let Some(dialog) = &mut editor.export else {
        return;
    };
    let mut options = editor.preferences.export;
    let items: Vec<_> = editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .filter(|item| editor.selection.contains(&item.id))
        .collect();
    dialog.selection_range = items
        .iter()
        .map(|item| item.start)
        .min()
        .zip(items.iter().filter_map(|item| item.end().ok()).max())
        .and_then(|(start, end)| time::TimeRange::new(start, end).ok());
    let output = dialog.ui(
        ctx,
        editor.preferences.locale,
        editor.preferences.dark,
        &mut options,
        editor.session.project(),
        editor.player.texture.as_ref(),
    );
    match output.action {
        Some(crate::export::Action::Start) => {
            let result = if editor.session.transaction_active() {
                Err(Error::Engine(engine::EditError::TransactionActive))
            } else {
                dialog.start(editor.session.project().clone(), options)
            };
            if let Err(error) = result {
                dialog.failed(error);
            }
        }
        Some(crate::export::Action::Retry) => {
            let result = if editor.session.transaction_active() {
                Err(Error::Engine(engine::EditError::TransactionActive))
            } else {
                dialog.retry(editor.session.project().clone())
            };
            if let Err(error) = result {
                dialog.failed(error);
            }
        }
        Some(crate::export::Action::Save) => {
            editor.lifecycle_action = Some(crate::lifecycle::EditorAction::ExportSave);
        }
        None => {}
    }
    if let Some(request) =
        dialog.take_save_request(editor.session.project(), editor.preferences.export)
    {
        let _ = request;
        editor.lifecycle_action = Some(crate::lifecycle::EditorAction::ExportSave);
    }
    if let Some(saved) = dialog.take_saved() {
        editor.preferences.export_directory = saved.parent().map(Path::to_owned);
        editor.set_toast(
            editor.tr(Text::ExportSaved),
            Some(saved.to_string_lossy().into_owned()),
        );
    }
    if options != editor.preferences.export {
        editor.desk().dispatch(
            PREFERENCE_COMMAND,
            serde_json::json!({"key": "export", "value": options}),
        );
    }
    for (id, response) in output.widgets {
        editor.remember(id, &response);
    }
}
