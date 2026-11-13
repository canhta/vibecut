//! Lends process resources to caption jobs.
use super::Error as SubtitleError;
use crate::runtime::{Error as RuntimeError, Result as RuntimeResult};
use crate::{
    cache::ArtifactCache,
    providers::{ConsentPrompt, Providers},
};
use std::any::Any;
use std::sync::{Arc, Mutex};

struct Resources {
    providers: Arc<Providers>,
    cache: Arc<Mutex<ArtifactCache>>,
}
/// Process-owned providers, exact human approval and one checkpoint-cache handle.
/// Missing optional resources retain their typed setup error without preventing
/// unrelated control commands from starting. No command supplies credentials or approval.
#[derive(Clone)]
pub struct CaptionAccess {
    resources: std::result::Result<Arc<Resources>, Arc<SubtitleError>>,
    consent: Arc<dyn ConsentPrompt + Send + Sync>,
    /// An opaque workflow lent by the composition root; the control layer, which
    /// names its type, stores and reads it through `with_clips` and `clips`.
    pub(crate) clips: Option<Arc<dyn Any + Send + Sync>>,
    /// The narration workflow, stored and read the same way through `with_dubbing`.
    pub(crate) dubbing: Option<Arc<dyn Any + Send + Sync>>,
}
impl CaptionAccess {
    pub fn new(
        providers: Arc<Providers>,
        consent: Arc<dyn ConsentPrompt + Send + Sync>,
        cache: Arc<Mutex<ArtifactCache>>,
    ) -> Self {
        Self {
            resources: Ok(Arc::new(Resources { providers, cache })),
            consent,
            clips: None,
            dubbing: None,
        }
    }
    /// Retain an optional composition failure for caption commands; other commands stay usable.
    pub fn unavailable(error: SubtitleError) -> Self {
        Self {
            resources: Err(Arc::new(error)),
            consent: Arc::new(crate::providers::DenyUploads),
            clips: None,
            dubbing: None,
        }
    }
    fn resources(&self) -> RuntimeResult<&Resources> {
        match &self.resources {
            Ok(resources) => Ok(resources),
            Err(error) => Err(RuntimeError::Failed(error.clone())),
        }
    }
    /// Reuse this cache for every workflow in the process; never open its root twice.
    pub fn cache(&self) -> RuntimeResult<&Arc<Mutex<ArtifactCache>>> {
        Ok(&self.resources()?.cache)
    }
    /// The selected providers and the human approval they ask, for workflows above the engine.
    pub fn share(&self) -> RuntimeResult<(Arc<Providers>, Arc<dyn ConsentPrompt + Send + Sync>)> {
        Ok((self.resources()?.providers.clone(), self.consent.clone()))
    }
}
