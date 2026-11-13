//! The installed local face detector a composition root lends to shot analysis.
use ai::foundation::models::Model;
use engine::{cache::ModelLease, control::VisionAccess};
use std::sync::Arc;

const MODEL: &str = "yunet-2026may";

/// The root's verified model-cache lookup. Each process opens its models cache once and
/// every consumer (model manager, captions, this loader) reads through that one owner, so
/// the loader never opens a second cache of its own.
pub type ModelLookup = Arc<
    dyn Fn(&Model, &dyn ai::ExecutionHooks) -> engine::control::Result<Option<ModelLease>>
        + Send
        + Sync,
>;

/// The local YuNet detector behind `reframe.generate` and clip-export layouts. Model files
/// come from the root's `lookup` and are never downloaded here: a missing bundle returns
/// the typed `ModelMissing` refusal, which clip analysis reports as `models_missing` and
/// the CLI reframe command turns into the `model download yunet-2026may` setup message.
pub fn installed_faces(lookup: ModelLookup) -> VisionAccess {
    VisionAccess::new(move || {
        let model = engine::models::catalog_model(MODEL)?;
        let hooks = Hooks;
        let lease = lookup(&model, &hooks)?.ok_or(engine::control::Error::Providers(
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
