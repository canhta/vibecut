//! Applies appearance drafts.

use crate::editor::properties::{ColorTarget, Editor, Error, Picker, Result};

impl Editor {
    /// Open the shared picker. Drafts preview through a temporary engine Session;
    /// the editable Session, history and persistence remain untouched until release.
    pub fn open_color_picker(&mut self, target: ColorTarget) -> Result<()> {
        self.close_color_picker();
        self.finish_scrub(false);
        let color = target.color(self.session.project())?;
        self.color_base = Some(self.session.project().clone());
        self.color_picker = Some(Picker::new(target, color));
        Ok(())
    }
    /// Inject the native capability boundary for an open picker.
    pub fn set_eyedropper(&mut self, sampler: crate::color_picker::NativeEyedropper) -> Result<()> {
        self.color_picker
            .as_mut()
            .ok_or(Error::Invalid("closed colour picker"))?
            .native = sampler;
        Ok(())
    }
    pub(in crate::editor) fn close_color_picker(&mut self) {
        if self.color_picker.take().is_some() {
            self.color_base = None;
            self.sync_preview();
        }
    }
}
