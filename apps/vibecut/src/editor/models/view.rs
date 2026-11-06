//! Renders the models window.
use crate::editor::{Editor, egui};

pub(in crate::editor) fn show(editor: &mut Editor, ctx: &egui::Context) {
    if !editor.models_visible {
        return;
    }
    let Some(models) = &mut editor.models else {
        return;
    };
    let output = models.ui(ctx, editor.preferences.locale, editor.preferences.dark);
    for (id, response) in output.widgets {
        editor.remember(id, &response);
    }
}
