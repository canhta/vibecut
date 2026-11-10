//! Keeps the model manager the editor reuses between frames.
use super::model_manager::{ModelInspection, ModelManager};
use crate::{Error, Result};
use engine::ByteBudget;

/// The model manager and whether its window owns the frame.
pub(in crate::editor) struct State {
    manager: Option<ModelManager>,
    visible: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            manager: None,
            visible: true,
        }
    }
}

impl State {
    /// Instantiate the one shared model cache on first use; it never fetches then.
    pub(in crate::editor) fn open(&mut self, budget: ByteBudget) -> Result<()> {
        self.ensure(budget)?;
        if let Some(manager) = &mut self.manager {
            manager.show();
        }
        Ok(())
    }

    /// Open the manager on a specific missing model.
    pub(in crate::editor) fn open_model(&mut self, budget: ByteBudget, id: &str) -> Result<()> {
        self.ensure(budget)?;
        self.manager
            .as_mut()
            .ok_or(Error::Invalid("model manager unavailable"))?
            .open_model(id)
    }

    /// Inject cache/transport/IO ownership for composition and tests.
    pub(in crate::editor) fn set(&mut self, manager: ModelManager) {
        self.manager = Some(manager);
    }

    /// The shared service, handed back to its app owner between windows.
    pub(in crate::editor) fn take(&mut self) -> Option<ModelManager> {
        self.manager.take()
    }

    pub(in crate::editor) fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    pub(in crate::editor) fn manager(&self) -> Option<&ModelManager> {
        self.manager.as_ref()
    }

    /// The manager, for the frame to render.
    pub(in crate::editor) fn manager_mut(&mut self) -> Option<&mut ModelManager> {
        self.manager.as_mut()
    }

    /// Whether this window currently presents the manager.
    pub(in crate::editor) fn visible(&self) -> bool {
        self.visible
    }

    pub(in crate::editor) fn inspect(&self) -> Option<ModelInspection> {
        self.manager.as_ref().map(ModelManager::inspect)
    }

    /// Whether the manager modal owns the keyboard this frame.
    pub(in crate::editor) fn is_open(&self) -> bool {
        self.visible
            && self
                .manager
                .as_ref()
                .is_some_and(|manager| manager.inspect().open)
    }

    fn ensure(&mut self, budget: ByteBudget) -> Result<()> {
        if self.manager.is_none() {
            self.manager = Some(ModelManager::open_default(budget)?);
        }
        Ok(())
    }
}
