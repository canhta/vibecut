//! Home window: the project library with its create, search, sort and open flows.
//!
//! The interface is [`State`] and [`State::show`]; `show` draws the whole
//! window and returns the one [`Action`] the workspace must carry out. Home
//! imports only the kit, the catalog and the engine's project library.

mod preferences;
mod state;
mod view;
mod workflow;

use crate::{appearance::Look, egui, i18n::Text};

pub use preferences::{ProjectsPreferences, ProjectsSortKey, ProjectsView};
pub use state::{Action, State};
/// The window Home opens at; the editor and shorts windows own their own sizes.
pub const WINDOW: [f32; 2] = [1728.0, 1030.0];

impl State {
    /// Draw the window and return the lifecycle action a click requested.
    /// `status` is the one-line short-video job state shown in the top band.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        look: Look,
        projects: &mut ProjectsPreferences,
        status: Option<Text>,
    ) -> Option<Action> {
        view::show(self, ui, look, projects, status);
        self.action.take()
    }
}
