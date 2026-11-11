//! Keeps the export dialog the editor reuses between frames.
use super::dialog::{ExportDialog, ExportFormat, ExportInspection, ExportOptions};
use crate::{Error, Result, services::playback::RenderState};
use project::Project;
use std::path::PathBuf;
use std::sync::Arc;

/// The export dialog, created lazily on first open.
#[derive(Default)]
pub(in crate::editor) struct State {
    dialog: Option<ExportDialog>,
}

impl State {
    /// Open or raise the dialog over the current project.
    pub(in crate::editor) fn open(
        &mut self,
        budget: engine::ByteBudget,
        render: Arc<RenderState>,
        project: &Project,
    ) -> Result<()> {
        self.ensure(budget, render)?;
        if let Some(dialog) = &mut self.dialog {
            dialog.show(project);
        }
        Ok(())
    }

    /// Start with typed caller settings without mutating editable project content.
    pub(in crate::editor) fn run(
        &mut self,
        project: &Arc<Project>,
        options: ExportOptions,
        budget: engine::ByteBudget,
        render: Arc<RenderState>,
    ) -> Result<()> {
        self.ensure(budget, render)?;
        let dialog = self
            .dialog
            .as_mut()
            .ok_or(Error::Invalid("export dialog"))?;
        dialog.show(project);
        dialog.start(project.clone(), options)
    }

    /// Start the attempt, propagating a refusal for the caller to report.
    pub(in crate::editor) fn start(
        &mut self,
        project: Arc<Project>,
        options: ExportOptions,
        blocked: bool,
    ) -> Result<()> {
        if blocked {
            return Err(Error::Engine(engine::control::Error::Session(
                engine::SessionError::TransactionActive,
            )));
        }
        let dialog = self
            .dialog
            .as_mut()
            .ok_or(Error::Invalid("export dialog"))?;
        dialog.start(project, options)
    }

    /// Retry the failed attempt, propagating a refusal for the caller to report.
    pub(in crate::editor) fn retry(&mut self, project: Arc<Project>, blocked: bool) -> Result<()> {
        if blocked {
            return Err(Error::Engine(engine::control::Error::Session(
                engine::SessionError::TransactionActive,
            )));
        }
        let dialog = self
            .dialog
            .as_mut()
            .ok_or(Error::Invalid("export dialog"))?;
        dialog.retry(project)
    }

    /// Show a failed request on the dialog's error panel.
    pub(in crate::editor) fn fail(&mut self, error: Error) {
        if let Some(dialog) = &mut self.dialog {
            dialog.failed(error);
        }
    }

    /// Accept the native save picker's result without opening OS UI in tests.
    pub(in crate::editor) fn save_to(
        &mut self,
        path: PathBuf,
        protected: Vec<PathBuf>,
    ) -> Result<()> {
        let dialog = self
            .dialog
            .as_mut()
            .ok_or(Error::Invalid("export dialog closed"))?;
        if let Err(error) = dialog.save_to(path, protected) {
            dialog.failed(error);
        }
        Ok(())
    }

    /// Cancelling the native save dialog keeps the finished job for a later save.
    pub(in crate::editor) fn save_cancelled(&mut self) {
        if let Some(dialog) = &mut self.dialog {
            dialog.save_cancelled();
        }
    }

    /// Default filename for the native save dialog.
    pub(in crate::editor) fn save_name(
        &self,
        project: &Project,
        options: ExportOptions,
    ) -> Option<String> {
        self.dialog
            .as_ref()
            .map(|dialog| dialog.save_name(project, options))
    }

    /// Default filename and last directory for the native save dialog.
    pub(in crate::editor) fn defaults(
        &self,
        project: &Project,
        options: ExportOptions,
        directory: Option<PathBuf>,
    ) -> (String, Option<PathBuf>) {
        let name = self.save_name(project, options).unwrap_or_else(|| {
            super::dialog::default_file_name(
                project.data().name.as_str(),
                options.format.extension(),
            )
        });
        (name, directory)
    }

    /// The completed attempt owns its container even if settings changed after failure.
    pub(in crate::editor) fn save_format(&self, fallback: ExportFormat) -> ExportFormat {
        self.dialog
            .as_ref()
            .and_then(|dialog| dialog.inspect().attempt)
            .map_or(fallback, |options| options.format)
    }

    /// Advance the running job and publish its terminal state.
    pub(in crate::editor) fn tick(&mut self, project: &Arc<Project>, options: ExportOptions) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        if let Err(error) = dialog.tick(project, options) {
            dialog.failed(error);
        }
    }

    pub(in crate::editor) fn inspect(&self) -> Option<ExportInspection> {
        self.dialog.as_ref().map(ExportDialog::inspect)
    }

    /// Whether the export modal owns the keyboard this frame.
    pub(in crate::editor) fn is_open(&self) -> bool {
        self.dialog.as_ref().is_some_and(|dialog| dialog.open)
    }

    /// The dialog, for state the workflow entry does not own.
    pub(in crate::editor) fn dialog_mut(&mut self) -> Option<&mut ExportDialog> {
        self.dialog.as_mut()
    }

    /// The dialog, for automation reads.
    pub(in crate::editor) fn dialog(&self) -> Option<&ExportDialog> {
        self.dialog.as_ref()
    }

    fn ensure(&mut self, budget: engine::ByteBudget, render: Arc<RenderState>) -> Result<()> {
        if self.dialog.is_none() {
            self.dialog = Some(ExportDialog::new(budget, render)?);
        }
        Ok(())
    }
}
