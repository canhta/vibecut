//! Keeps the model manager state alongside the requests it returns.
pub mod model_manager;
mod state;
#[cfg(test)]
mod tests;
mod view;

pub(in crate::editor) use state::State;

use self::model_manager::ModelManager;
use crate::editor::{Editor, Result, egui};

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

pub(in crate::editor) fn apply(desk: &mut crate::editor::Desk<'_>, action: Action) {
    view::apply(desk, action);
}

impl Editor {
    /// Instantiate the one shared model cache on first use; it never fetches then.
    pub fn open_models(&mut self) -> Result<()> {
        let budget = self.playback.budget().clone();
        self.dialogs.models.open(budget)
    }

    /// Open the manager on a specific missing model, as `/09` does.
    pub fn open_model(&mut self, id: &str) -> Result<()> {
        let budget = self.playback.budget().clone();
        self.dialogs.models.open_model(budget, id)
    }

    /// Inject cache/transport/IO ownership for composition and tests.
    pub fn set_model_manager(&mut self, models: ModelManager) {
        self.dialogs.models.set(models);
    }

    pub(crate) fn take_model_manager(&mut self) -> Option<ModelManager> {
        self.dialogs.models.take()
    }

    pub(crate) fn set_models_visible(&mut self, visible: bool) {
        self.dialogs.models.set_visible(visible);
    }

    pub fn models(&self) -> Option<&ModelManager> {
        self.dialogs.models.manager()
    }
}
