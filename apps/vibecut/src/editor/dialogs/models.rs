//! Keeps the model manager state alongside the requests it returns.
pub mod model_manager;
mod state;
#[cfg(test)]
mod tests;
mod view;

pub(in crate::editor) use state::State;

use crate::editor::egui;

/// What the frame must do after the model manager painted.
pub(in crate::editor) enum Action {
    Remember {
        id: String,
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

pub(in crate::editor) fn apply(port: &mut crate::editor::Port<'_>, action: Action) {
    view::apply(port, action);
}
