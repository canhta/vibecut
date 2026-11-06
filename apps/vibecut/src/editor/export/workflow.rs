//! Applies export requests.
use crate::editor::{Editor, Error, PathBuf, Result};

impl Editor {
    /// Start with typed caller settings without mutating editable project content.
    pub(in crate::editor) fn run_export(
        &mut self,
        options: crate::export::ExportOptions,
    ) -> Result<()> {
        if self.session.transaction_active() {
            return Err(Error::Engine(engine::EditError::TransactionActive));
        }
        if self.export.is_none() {
            self.export = Some(crate::export::ExportDialog::new(
                self.playback.budget().clone(),
                self.playback.render_state()?,
            )?);
        }
        let dialog = self
            .export
            .as_mut()
            .ok_or(Error::Invalid("export dialog"))?;
        dialog.show(self.session.project());
        dialog.start(self.session.project().clone(), options)
    }
    /// Accept the native export save picker's result without opening OS UI in tests.
    pub fn export_picked_file(&mut self, path: PathBuf) -> Result<()> {
        let protected = self
            .session
            .autosave()
            .map(|s| s.protected_export_paths())
            .transpose()?
            .unwrap_or_default();
        let dialog = self
            .export
            .as_mut()
            .ok_or(Error::Invalid("export dialog closed"))?;
        if let Err(error) = dialog.save_to(path, protected) {
            dialog.failed(error);
        }
        Ok(())
    }

    /// Cancelling the native save dialog keeps the finished job for a later save.
    pub fn export_save_cancelled(&mut self) {
        if let Some(dialog) = &mut self.export {
            dialog.save_cancelled();
        }
    }
    /// Default filename and last directory for the native save dialog.
    pub fn export_save_defaults(&self) -> (String, Option<PathBuf>) {
        (
            self.export.as_ref().map_or_else(
                || {
                    crate::export::default_file_name(
                        self.session.project().data().name.as_str(),
                        self.export_save_format().extension(),
                    )
                },
                |dialog| dialog.save_name(self.session.project(), self.preferences.export),
            ),
            self.preferences.export_directory.clone(),
        )
    }
    /// The completed attempt owns its container even if settings changed after failure.
    pub fn export_save_format(&self) -> crate::export::ExportFormat {
        self.export
            .as_ref()
            .and_then(|dialog| dialog.inspect().attempt)
            .map_or(self.preferences.export.format, |options| options.format)
    }
}
