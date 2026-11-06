//! Applies colour picker requests.

use crate::editor::properties::{ColorTarget, Editor, Error, Result, Session, Tick, apply};

impl Editor {
    /// Open the shared picker. Drafts preview through a temporary engine Session;
    /// the editable Session, history and persistence remain untouched until release.
    pub fn open_color_picker(&mut self, target: ColorTarget) -> Result<()> {
        let actions = self
            .properties
            .open_picker(self.session.project(), target, None)?;
        apply(self, actions);
        Ok(())
    }
    /// Inject the native capability boundary for an open picker.
    pub fn set_eyedropper(&mut self, sampler: crate::color_picker::NativeEyedropper) -> Result<()> {
        self.properties
            .picker
            .as_mut()
            .ok_or(Error::Invalid("closed colour picker"))?
            .native = sampler;
        Ok(())
    }
    /// Show a colour draft without touching the editable session.
    pub(in crate::editor::properties) fn preview_color(
        &mut self,
        command: &'static str,
        params: serde_json::Value,
    ) -> Result<()> {
        let result = (|| -> Result<()> {
            let mut draft = Session::new(self.session.project().as_ref().clone(), 1)?;
            draft.execute(command, params)?;
            let at = self.playback.position().unwrap_or(Tick::ZERO);
            self.playback.seek(at, false, draft.project().clone())?;
            Ok(())
        })();
        if result.is_err() {
            self.sync_preview();
        }
        result
    }
    /// Commit a released colour as one undoable command and remember it as recent.
    pub(in crate::editor::properties) fn commit_color(
        &mut self,
        command: &'static str,
        params: serde_json::Value,
        recent: Vec<project::Color>,
    ) -> Result<()> {
        let result = (|| -> Result<()> {
            let mut preferences = self.preferences.clone();
            preferences.set("recent_colors", serde_json::to_value(recent)?)?;
            self.session.end_merge();
            self.session.execute(command, params)?;
            if let Some(picker) = self.properties.picker.as_mut() {
                picker.original = picker.color;
            }
            self.properties.color_base = Some(self.session.project().clone());
            self.preferences = preferences;
            self.sync_preview();
            Ok(())
        })();
        if result.is_err() {
            self.sync_preview();
        }
        result
    }
}
