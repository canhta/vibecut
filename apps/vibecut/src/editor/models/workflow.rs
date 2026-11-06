//! Applies models requests.
use crate::editor::{Editor, Error, Result};

impl Editor {
    /// Instantiate the one shared model cache on first use; it never fetches then.
    pub fn open_models(&mut self) -> Result<()> {
        if self.models.is_none() {
            self.models = Some(crate::model_manager::ModelManager::open_default(
                self.playback.budget().clone(),
            )?);
        }
        if let Some(models) = &mut self.models {
            models.show();
        }
        Ok(())
    }
    /// Open the manager on a specific missing model, as `/09` does.
    pub fn open_model(&mut self, id: &str) -> Result<()> {
        if self.models.is_none() {
            self.models = Some(crate::model_manager::ModelManager::open_default(
                self.playback.budget().clone(),
            )?);
        }
        self.models
            .as_mut()
            .ok_or(Error::Invalid("model manager unavailable"))?
            .open_model(id)
    }
    /// Inject cache/transport/IO ownership for composition and tests.
    pub fn set_model_manager(&mut self, models: crate::model_manager::ModelManager) {
        self.models = Some(models);
    }
    pub(crate) fn take_model_manager(&mut self) -> Option<crate::model_manager::ModelManager> {
        self.models.take()
    }
    pub(crate) fn set_models_visible(&mut self, visible: bool) {
        self.models_visible = visible;
    }
    pub fn models(&self) -> Option<&crate::model_manager::ModelManager> {
        self.models.as_ref()
    }
}
