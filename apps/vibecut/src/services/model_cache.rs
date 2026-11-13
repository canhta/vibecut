//! The process models cache: one `ModelTransfers` shared by the model manager and the face detector.
use crate::{Error, Result};
use ai::foundation::models::Model;
use engine::{ByteBudget, cache::ModelLease, models::ModelTransfers};
use std::sync::{Arc, Mutex};

/// Opens the models cache on first use and hands every consumer the same handle. The cache
/// root is exclusive, so a second `ModelTransfers` would fail busy while the first lives.
pub struct ModelCache {
    budget: ByteBudget,
    opened: Mutex<Option<Arc<ModelTransfers>>>,
}

impl ModelCache {
    pub fn new(budget: ByteBudget) -> Self {
        Self {
            budget,
            opened: Mutex::new(None),
        }
    }
    /// A cache over an already-opened `ModelTransfers`, for composition and tests.
    pub fn with_transfers(budget: ByteBudget, transfers: ModelTransfers) -> Self {
        Self {
            budget,
            opened: Mutex::new(Some(Arc::new(transfers))),
        }
    }
    pub fn budget(&self) -> &ByteBudget {
        &self.budget
    }
    /// The shared handle; the first call opens the default models root. It never fetches.
    pub fn transfers(&self) -> Result<Arc<ModelTransfers>> {
        let mut opened = self.opened.lock().map_err(|_| Error::Poisoned)?;
        if let Some(transfers) = &*opened {
            return Ok(transfers.clone());
        }
        let (transfers, _) = ModelTransfers::open_native(self.budget.clone(), 128)
            .map_err(engine::control::Error::from)?;
        let transfers = Arc::new(transfers);
        *opened = Some(transfers.clone());
        Ok(transfers)
    }
    /// Installed-only lookup for `shorts::installed_faces`, through the shared handle.
    pub fn lookup(self: &Arc<Self>) -> ::shorts::ModelLookup {
        let cache = self.clone();
        Arc::new(
            move |model: &Model,
                  hooks: &dyn ai::ExecutionHooks|
                  -> engine::control::Result<Option<ModelLease>> {
                let transfers = cache.transfers().map_err(|error| match error {
                    Error::Engine(error) => error,
                    _ => engine::RuntimeError::Poisoned.into(),
                })?;
                Ok(transfers.installed(model, hooks)?)
            },
        )
    }
}
