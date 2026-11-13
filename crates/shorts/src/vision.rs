//! The installed local face detector a composition root lends to shot analysis.
use engine::{ByteBudget, cache::ArtifactCache, control::VisionAccess};
use std::sync::{Arc, Mutex};

const MODEL: &str = "yunet-2026may";

/// The local YuNet detector behind `reframe.generate` and clip-export layouts. Model files
/// are read from the shared verified cache and never downloaded here: a missing bundle
/// returns the typed `ModelMissing` refusal, which clip analysis reports as `models_missing`
/// and the CLI reframe command turns into the `model download yunet-2026may` setup message.
/// A root that already holds the models cache (through `CaptionAccess::cache`) passes it as
/// `shared`, because that root is opened once per process; otherwise each load opens the
/// cache for the duration of the lookup.
pub fn installed_faces(
    budget: ByteBudget,
    shared: Option<Arc<Mutex<ArtifactCache>>>,
) -> VisionAccess {
    VisionAccess::new(move || {
        let model = engine::models::catalog_model(MODEL)?;
        let hooks = Hooks;
        let lease = match &shared {
            Some(cache) => cache
                .lock()
                .map_err(|_| engine::RuntimeError::Poisoned)?
                .model(&model, &hooks)?,
            None => {
                let (transfers, _) =
                    engine::models::ModelTransfers::open_native(budget.clone(), 128)?;
                transfers.installed(&model, &hooks)?
            }
        }
        .ok_or(engine::control::Error::Providers(
            engine::providers::Error::ModelMissing(MODEL),
        ))?;
        let vision = ai::vision::TractVision::load_faces(lease.directory(), 2, 3, &hooks)?;
        let fingerprint = *vision.fingerprint()?.as_bytes();
        Ok((
            Box::new(vision) as Box<dyn ai::vision::VisionDetector>,
            fingerprint,
        ))
    })
}

struct Hooks;
impl ai::ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn stage(&self, _: ai::Stage) {}
}
