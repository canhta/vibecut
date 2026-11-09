//! Renders the captions panel.
use crate::editor::{Arc, Editor, Error, Request, Tick, egui};

pub(in crate::editor) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    if let Err(e) = editor.prepare_subtitles() {
        editor.desk().fail(e.to_string());
        return;
    }
    let before = editor.session.project().clone();
    let mut options = editor.preferences.subtitles.clone();
    let at = editor.playback.position().unwrap_or(Tick::ZERO);
    if let Some(subtitles) = &mut editor.subtitles {
        let widgets = subtitles.ui(
            ui,
            &mut editor.session,
            &mut options,
            &editor.selection,
            at,
            editor.preferences.locale,
            editor.preferences.dark,
        );
        for (id, response) in widgets {
            editor.remember(id, &response);
        }
    }
    queue_requests(editor);
    if options != editor.preferences.subtitles {
        match serde_json::to_value(options)
            .map_err(Error::from)
            .and_then(|value| editor.set_preference("subtitles", value))
        {
            Ok(()) => {}
            Err(e) => editor.desk().fail(e.to_string()),
        }
    }
    if !Arc::ptr_eq(&before, editor.session.project()) {
        editor.desk().sync();
    }
}

/// Move the panel's cross-area intentions into the frame's request queue.
pub(in crate::editor) fn queue_requests(editor: &mut Editor) {
    let mut desk = editor.desk();
    let seek = desk
        .editor()
        .subtitles
        .as_mut()
        .and_then(|panel| panel.requested_seek.take());
    let model = desk
        .editor()
        .subtitles
        .as_mut()
        .and_then(|panel| panel.requested_model.take());
    if let Some(at) = seek {
        desk.request(Request::Seek(at));
    }
    if let Some(model) = model {
        desk.request(Request::OpenModel(model));
    }
}
