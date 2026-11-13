//! Home window: the project library with its create, search, sort and open flows.
//!
//! The window interface is [`State`], [`Context`], [`Action`] and [`show`]:
//! `show` draws the whole window and returns the [`Action`]s the composition
//! root must carry out. Home imports only the kit, the catalog and the
//! engine's project library.

mod preferences;
mod state;
mod view;
mod workflow;

use crate::{egui, i18n::Text, kit::Look};

pub use preferences::{ProjectsPreferences, ProjectsSortKey, ProjectsView};
pub use state::{Action, State};
pub use workflow::Catalog;
/// The window Home opens at; the editor and shorts windows own their own sizes.
pub const WINDOW: [f32; 2] = [1728.0, 1030.0];

/// Facts and shared preferences the composition root lends Home for one frame.
pub struct Context<'a> {
    pub look: Look,
    pub projects: &'a mut ProjectsPreferences,
    /// The one-line short-video job state shown in the top band.
    pub status: Option<Text>,
    pub default_fps: u32,
}

/// Draw the window and return the lifecycle actions a click requested.
pub fn show(state: &mut State, cx: Context<'_>, ui: &mut egui::Ui) -> Vec<Action> {
    state.default_fps = cx.default_fps;
    view::show(state, ui, cx.look, cx.projects, cx.status);
    state.action.take().into_iter().collect()
}
