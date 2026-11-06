//! Owns preview presentation state behind the player panel interface.
mod state;
mod view;
mod workflow;

use crate::{editor::Context, egui};
pub(super) use state::State;

/// Requests are applied by the frame after rendering releases its read borrow.
pub(super) enum Action {
    Command(String),
    Copy,
    Retry,
    CommitTimecode,
    Zoom(f32),
    Remember {
        id: String,
        icon: Option<&'static str>,
        tooltip: Option<String>,
        response: egui::Response,
    },
}

pub(super) fn show(state: &mut State, cx: &Context<'_>, ui: &mut egui::Ui) -> Vec<Action> {
    view::show(state, cx, ui)
}
