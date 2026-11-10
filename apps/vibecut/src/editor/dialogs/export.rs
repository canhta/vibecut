//! Keeps the export sheet state alongside the requests it returns.
mod dialog;
mod settings;
mod state;
#[cfg(test)]
mod tests;
mod view;

pub use dialog::{
    ExportDialog, ExportFormat, ExportInspection, ExportOptions, ExportPhase, ExportQuality,
    OutputOptions, RangeChoice, SaveRequest, SceneChoice, default_file_name, sanitize_file_name,
};
pub(in crate::editor) use state::State;

use crate::editor::{Editor, Error, Result, egui};
use std::path::PathBuf;

/// What the frame must do after the export dialog painted.
pub(in crate::editor) enum Action {
    Start(ExportOptions),
    Retry,
    Save,
    Options(ExportOptions),
    Saved(PathBuf),
    Remember {
        id: &'static str,
        response: egui::Response,
    },
}

pub(in crate::editor) fn show(
    state: &mut State,
    cx: &crate::editor::Context<'_>,
    ui: &mut egui::Ui,
) -> Vec<Action> {
    view::show(state, cx, ui.ctx())
}

pub(in crate::editor) fn apply(desk: &mut crate::editor::Desk<'_>, action: Action) {
    view::apply(desk, action);
}

impl Editor {
    /// Start with typed caller settings without mutating editable project content.
    pub(in crate::editor) fn run_export(&mut self, options: ExportOptions) -> Result<()> {
        if self.session.transaction_active() {
            return Err(Error::Engine(engine::EditError::TransactionActive));
        }
        let project = self.session.project().clone();
        let budget = self.playback.budget().clone();
        let render = self.playback.render_state()?;
        self.dialogs.export.run(&project, options, budget, render)
    }

    /// Open or raise the dialog without changing project content.
    pub(in crate::editor) fn open_export(&mut self) -> Result<()> {
        let budget = self.playback.budget().clone();
        let render = self.playback.render_state()?;
        let project = self.session.project().clone();
        self.dialogs.export.open(budget, render, &project)
    }

    /// Accept the native export save picker's result without opening OS UI in tests.
    pub fn export_picked_file(&mut self, path: PathBuf) -> Result<()> {
        let protected = self
            .session
            .autosave()
            .map(|s| s.protected_export_paths())
            .transpose()?
            .unwrap_or_default();
        self.dialogs.export.save_to(path, protected)
    }

    /// Cancelling the native save dialog keeps the finished job for a later save.
    pub fn export_save_cancelled(&mut self) {
        self.dialogs.export.save_cancelled();
    }

    /// Default filename and last directory for the native save dialog.
    pub fn export_save_defaults(&self) -> (String, Option<PathBuf>) {
        self.dialogs.export.defaults(
            self.session.project(),
            self.preferences.export,
            self.preferences.export_directory.clone(),
        )
    }

    /// The completed attempt owns its container even if settings changed after failure.
    pub fn export_save_format(&self) -> ExportFormat {
        self.dialogs
            .export
            .save_format(self.preferences.export.format)
    }

    /// The export dialog, for automation and test injection.
    pub fn export_dialog(&mut self) -> Option<&mut ExportDialog> {
        self.dialogs.export.dialog_mut()
    }

    /// The export dialog state, for automation reads.
    pub fn export_state(&self) -> Option<&ExportDialog> {
        self.dialogs.export.dialog()
    }
}
