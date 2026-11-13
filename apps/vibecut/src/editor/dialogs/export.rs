//! Keeps the export sheet state alongside the requests it returns.
mod caption_outputs;
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

use crate::editor::egui;

/// What the frame must do after the export dialog painted.
pub(in crate::editor) enum Action {
    Start(ExportOptions),
    Retry,
    Save,
    /// A caption file or the burned MP4, saved through the window's native picker.
    Caption(crate::services::lifecycle::CaptionOutput),
    Options(ExportOptions),
    Saved(std::path::PathBuf),
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

pub(in crate::editor) fn apply(
    state: &mut State,
    port: &mut crate::editor::Port<'_>,
    action: Action,
) {
    view::apply(state, port, action);
}
