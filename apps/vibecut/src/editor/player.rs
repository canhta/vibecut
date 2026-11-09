//! Owns preview presentation state behind the player panel interface.
mod state;
mod text;
mod transform;
mod view;
mod workflow;

use crate::{editor::Context, egui};
use project::ItemId;
pub(super) use state::State;

/// Requests are applied by the frame after rendering releases its read borrow.
pub(super) enum Action {
    Command(String),
    Copy,
    Retry,
    CommitTimecode,
    Zoom(f32),
    Select(ItemId),
    ClearSelection,
    /// Absolute transform values staged from one gesture origin.
    TransformPreview {
        item: ItemId,
        values: Vec<(project::TransformField, f64)>,
    },
    /// Close the staged gesture, publishing one undo step when `commit`.
    TransformEnd {
        commit: bool,
    },
    /// Open one inline text session and select its item.
    TextBegin {
        item: ItemId,
    },
    /// Stage the whole text buffer as the session's absolute candidate.
    TextPreview {
        item: ItemId,
        content: String,
    },
    /// Close the inline text session; `commit` publishes one undo step.
    TextEnd {
        commit: bool,
    },
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
