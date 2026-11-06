//! Renders the captions panel.
use crate::editor::{Arc, Editor, Error, Tick, egui};

pub(in crate::editor) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    if let Err(e) = editor.prepare_subtitles() {
        editor.error = Some(e.to_string());
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
        let seek = subtitles.requested_seek.take();
        let model = subtitles.requested_model.take();
        for (id, response) in widgets {
            editor.remember(id, &response);
        }
        if let Some(at) = seek
            && let Err(e) = editor.seek(at)
        {
            editor.error = Some(e.to_string());
        }
        if let Some(model) = model
            && let Err(e) = editor.open_model(&model)
        {
            editor.error = Some(e.to_string());
        }
    }
    if options != editor.preferences.subtitles {
        match serde_json::to_value(options)
            .map_err(Error::from)
            .and_then(|value| editor.set_preference("subtitles", value))
        {
            Ok(()) => {}
            Err(e) => editor.error = Some(e.to_string()),
        }
    }
    if !Arc::ptr_eq(&before, editor.session.project()) {
        editor.sync_preview();
    }
}
