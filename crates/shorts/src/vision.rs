//! The installed local vision models a composition root lends: the face detector behind shot
//! analysis and the person segmenter behind background removal.
use ai::foundation::models::Model;
use engine::{
    cache::ModelLease,
    control::{SegmentationAccess, VisionAccess},
};
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
/// A lookup that finds the cache in use (a model transfer runs) returns `CacheBusy` at once
/// and this loader passes it through unchanged, so analysis reports `models_busy`, never
/// missing weights; nothing here waits, because the caller may be a UI thread.
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

/// The local PP-HumanSeg segmenter behind `footage.analyze {kind: person_matte}`, at the
/// engine's fixed thread count. Weights come from the root's `lookup` and are never downloaded
/// here: a missing bundle returns the typed `ModelMissing` refusal, which the editor turns into
/// a link to the model manager.
pub fn installed_person_matte(lookup: ModelLookup) -> SegmentationAccess {
    SegmentationAccess::new(move || {
        let id = engine::footage::PERSON_MODEL;
        let model = engine::models::catalog_model(id)?;
        let hooks = Hooks;
        let lease = lookup(&model, &hooks)?.ok_or(engine::control::Error::Providers(
            engine::providers::Error::ModelMissing(id),
        ))?;
        let mut segmentation = ai::vision::TractSegmentation::load(
            lease.directory(),
            engine::footage::SEGMENTATION_THREADS,
            &hooks,
        )?;
        Ok(Box::new(
            move |frame: ai::vision::PersonFrame<'_>, hooks: &dyn ai::ExecutionHooks| {
                segmentation.matte(frame, hooks)
            },
        ) as engine::footage::Segmenter)
    })
}

struct Hooks;
impl ai::ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn stage(&self, _: ai::Stage) {}
}
