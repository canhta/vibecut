//! Mounts the captions panel in the assets area and queues its cross-area requests.
use super::Subtitles;
use crate::editor::panels::assets::State;
use crate::editor::{Arc, Error, Port, Request, Result, Tick, egui};
use crate::services::lifecycle::{CaptionOutput, EditorAction};
use std::path::PathBuf;

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

    /// Paint the captions panel into `ui` and leave its edits, preference changes and
    /// cross-area requests on the port.
    pub(in crate::editor) fn show_captions(&mut self, port: &mut Port<'_>, ui: &mut egui::Ui) {
        let budget = port.read.budget;
        if let Err(e) = self.prepare_captions(budget) {
            port.fail(e.to_string());
            return;
        }
        let before = port.session.project().clone();
        let mut options = port.read.preferences.subtitles.clone();
        let at = port.read.playhead.unwrap_or(Tick::ZERO);
        let bank_ready = port.read.caption_bank_ready;
        let section = self.captions_section;
        let (locale, dark) = (port.locale(), port.dark());
        let models = port.read.preferences.ai_models.clone();
        if let Some(subtitles) = &mut self.captions {
            subtitles.models = models;
            let widgets = subtitles.ui(
                ui,
                &mut *port.session,
                &mut options,
                at,
                locale,
                dark,
                bank_ready,
                section,
            );
            for (id, response) in widgets {
                port.widget(id, None, None, &response);
            }
        }
        for request in self.caption_requests() {
            port.request(request);
        }
        if options != port.read.preferences.subtitles {
            match serde_json::to_value(options).map_err(Error::from) {
                Ok(value) => port.set_preference("subtitles", value),
                Err(e) => port.fail(e.to_string()),
            }
        }
        if !Arc::ptr_eq(&before, port.session.project()) {
            port.sync();
        }
    }

    /// Parse the SRT/VTT file the window's picker chose, at the playhead the import asked from.
    pub(in crate::editor) fn import_subtitle_picked(
        &mut self,
        port: &mut Port<'_>,
        path: PathBuf,
        at: Tick,
    ) {
        let budget = port.read.budget;
        if let Err(e) = self.prepare_captions(budget) {
            port.fail(e.to_string());
            return;
        }
        let options = port.read.preferences.subtitles.clone();
        if let Some(subtitles) = &mut self.captions {
            subtitles.import_picked(&*port.session, path, &options.source, at);
        }
    }

    /// Save the export dialog's caption output to the picked destination. The panel runs the
    /// same engine export as before; the native save dialog already confirmed any replacement.
    pub(in crate::editor) fn export_caption_output(
        &mut self,
        port: &mut Port<'_>,
        path: PathBuf,
        output: CaptionOutput,
    ) {
        let budget = port.read.budget;
        if let Err(e) = self.prepare_captions(budget) {
            port.fail(e.to_string());
            return;
        }
        let options = port.read.preferences.subtitles.clone();
        let Some(subtitles) = &mut self.captions else {
            return;
        };
        let result = match output {
            CaptionOutput::File(format) => {
                subtitles.export_subtitles(&*port.session, path, format, &options, true)
            }
            CaptionOutput::Burned => subtitles.burn_subtitles(&*port.session, path, &options, true),
        };
        if let Err(e) = result {
            port.fail(e.to_string());
        }
    }

    /// Take the panel's cross-area intentions, each exactly once.
    pub(in crate::editor) fn caption_requests(&mut self) -> Vec<Request> {
        let Some(panel) = self.captions.as_mut() else {
            return Vec::new();
        };
        let mut requests = Vec::new();
        if let Some(at) = panel.requested_import.take() {
            requests.push(Request::Lifecycle(EditorAction::ImportSubtitles { at }));
        }
        if let Some(at) = panel.requested_seek.take() {
            requests.push(Request::Seek(at));
        }
        if let Some(model) = panel.requested_model.take() {
            requests.push(Request::OpenModel(model));
        }
        requests
    }
}
