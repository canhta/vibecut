//! Mounts the captions panel in the assets area and queues its cross-area requests.
use super::Subtitles;
use crate::editor::panels::assets::State;
use crate::editor::{Arc, Editor, Error, Request, Result, Tick, egui};

impl State {
    /// Initialize the panel without any download or provider construction.
    pub(in crate::editor) fn prepare_captions(
        &mut self,
        budget: &engine::ByteBudget,
    ) -> Result<()> {
        if self.captions.is_none() {
            self.captions = Some(Subtitles::new(budget.clone())?);
        }
        Ok(())
    }
}

pub(in crate::editor) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    let budget = editor.playback.budget().clone();
    if let Err(e) = editor.assets.prepare_captions(&budget) {
        editor.desk().fail(e.to_string());
        return;
    }
    let before = editor.session.project().clone();
    let mut options = editor.preferences.subtitles.clone();
    let at = editor.playback.position().unwrap_or(Tick::ZERO);
    let bank_ready = editor.caption_bank_ready();
    if let Some(subtitles) = &mut editor.assets.captions {
        subtitles.models = editor.preferences.ai_models.clone();
        let widgets = subtitles.ui(
            ui,
            &mut editor.session,
            &mut options,
            &editor.selection,
            at,
            editor.preferences.locale,
            editor.preferences.dark,
            bank_ready,
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
        .assets
        .captions
        .as_mut()
        .and_then(|panel| panel.requested_seek.take());
    let model = desk
        .editor()
        .assets
        .captions
        .as_mut()
        .and_then(|panel| panel.requested_model.take());
    if let Some(at) = seek {
        desk.request(Request::Seek(at));
    }
    if let Some(model) = model {
        desk.request(Request::OpenModel(model));
    }
}
