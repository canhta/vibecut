//! Applies colour picker requests.

use crate::editor::properties::{ColorTarget, Editor, Error, Result, Session, Tick, apply};

impl Editor {
    /// Open the shared picker. Drafts preview through a temporary engine Session;
    /// the editable Session, history and persistence remain untouched until release.
    pub fn open_color_picker(&mut self, target: ColorTarget) -> Result<()> {
        let actions = self
            .properties
            .open_picker(self.session.project(), target, None)?;
        apply(&mut self.desk(), actions);
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
        command: engine::Command,
    ) -> Result<()> {
        let result = (|| -> Result<()> {
            let mut draft = Session::new(self.session.project().as_ref().clone(), 1)?;
            draft.apply(command)?;
            let at = self.playback.position().unwrap_or(Tick::ZERO);
            self.playback.seek(at, false, draft.project().clone())?;
            Ok(())
        })();
        if result.is_err() {
            self.desk().sync();
        }
        result
    }
    /// Commit a released colour as one undoable command and remember it as recent.
    pub(in crate::editor::properties) fn commit_color(
        &mut self,
        command: engine::Command,
        recent: Vec<project::Color>,
    ) -> Result<()> {
        let mut preferences = self.preferences.clone();
        preferences.set("recent_colors", serde_json::to_value(recent)?)?;
        self.session.end_merge();
        if let Err(error) = self.desk().apply_result(command, false) {
            // The engine refused: show the committed project again.
            self.desk().sync();
            return Err(error);
        }
        if let Some(picker) = self.properties.picker.as_mut() {
            picker.original = picker.color;
        }
        self.properties.color_base = Some(self.session.project().clone());
        self.preferences = preferences;
        Ok(())
    }
}
