//! Renders the colour window.
use crate::editor::properties::{Editor, Result, Session, Tick, egui};

pub(in crate::editor) fn show_dialog(editor: &mut Editor, ctx: &egui::Context) {
    let Some(mut picker) = editor.color_picker.take() else {
        return;
    };
    let event = picker.ui(
        ctx,
        editor.preferences.locale,
        &editor.preferences.recent_colors,
    );
    for (id, response) in event.widgets {
        editor.remember(id, &response);
    }
    if event.cancel {
        editor.color_base = None;
        editor.sync_preview();
        return;
    }
    let result = (|| -> Result<()> {
        let (command, params) = picker
            .target
            .command(editor.session.project(), picker.color)?;
        if event.commit && picker.color != picker.original {
            let mut recent = editor.preferences.recent_colors.clone();
            recent.retain(|c| *c != picker.color);
            recent.insert(0, picker.color);
            recent.truncate(16);
            let mut preferences = editor.preferences.clone();
            preferences.set("recent_colors", serde_json::to_value(recent)?)?;
            editor.session.end_merge();
            editor.session.execute(command, params)?;
            picker.original = picker.color;
            editor.color_base = Some(editor.session.project().clone());
            editor.preferences = preferences;
            editor.sync_preview();
        } else if event.preview {
            let mut draft = Session::new(editor.session.project().as_ref().clone(), 1)?;
            draft.execute(command, params)?;
            let at = editor.playback.position().unwrap_or(Tick::ZERO);
            editor.playback.seek(at, false, draft.project().clone())?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        editor.error = Some(error.to_string());
        editor.sync_preview();
    }
    editor.color_picker = Some(picker);
}
