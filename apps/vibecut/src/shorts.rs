//! Long-video-to-shorts window: pick a video, review suggested shorts, export them.
//!
//! The window interface is [`State`], [`Context`], [`Action`] and [`show`]:
//! `show` draws the whole window and returns the [`Action`]s the composition
//! root must carry out. The window owns its own size, chrome and layout and
//! imports only the kit, the catalog and the engine.

mod state;
mod view;

use crate::egui;

pub use state::{
    ClipCandidate, Decision, GenerationRequest, Importer, Review, ReviewSource, State, TaskEvents,
    desktop_importer,
};

/// The window the shorts viewport opens at; Home and the editor own their own sizes.
pub const WINDOW: [f32; 2] = [1718.0, 989.0];

/// Facts from the composition root that the window cannot know.
#[derive(Clone, Copy)]
pub struct Context {
    /// False while another window is loading a project, so "open in editor" waits.
    pub editor_available: bool,
}

/// What the composition root must do for a click in the window.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Open the selected short as a project in the editor.
    OpenEditor,
}

/// Draw the window and return what its controls asked for.
pub fn show(state: &mut State, cx: Context, ui: &mut egui::Ui) -> Vec<Action> {
    view::show(state, &cx, ui)
}
