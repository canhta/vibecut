//! Configures the CLI engine runtime.
use crate::args::CliError;
use engine::ByteBudget;
use engine::jobs::{JobRunner, RunnerConfig, SystemClock};
use std::sync::Arc;
use std::time::Duration;

pub(crate) fn cli_runtime() -> Result<engine::providers::Runtime, CliError> {
    let mut runtime = engine::providers::Runtime::default();
    if let Ok(value) = std::env::var("VIBECUT_ASR_THREADS") {
        let threads = value.parse::<u8>().map_err(|_| {
            CliError::Usage("VIBECUT_ASR_THREADS must be an integer from 1 to 32".into())
        })?;
        if !(1..=32).contains(&threads) {
            return Err(CliError::Usage(
                "VIBECUT_ASR_THREADS must be an integer from 1 to 32".into(),
            ));
        }
        runtime.threads = threads;
    }
    if std::env::var("VIBECUT_ASR_GPU").is_ok_and(|value| value == "0") {
        runtime.use_gpu = false;
    }
    runtime.gipformer_runner = engine::providers::local_asr_runner();
    Ok(runtime)
}

pub(crate) fn cli_runner(budget: ByteBudget) -> Result<JobRunner, CliError> {
    cli_runner_sized(budget, 8)
}

/// A runner whose queue and retained results hold `slots` jobs, for a command that
/// submits a whole selection at once, such as one export job per short.
pub(crate) fn cli_runner_sized(budget: ByteBudget, slots: usize) -> Result<JobRunner, CliError> {
    Ok(JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 2],
            queue: slots,
            retained: slots,
            network_attempts: 3,
            backoff: Duration::from_millis(250),
            progress_interval: Duration::from_millis(50),
        },
        budget,
        Arc::new(SystemClock::default()),
    )?)
}

/// Optional local speech resources. Their typed refusal is lent to control while
/// edits and inspection remain available; this never downloads models.
pub(crate) fn caption_access(budget: ByteBudget) -> engine::subtitles::CaptionAccess {
    use engine::{
        cache::{ArtifactCache, CacheRoot},
        providers::{Credentials, DenyUploads, Providers},
    };
    let open = || -> engine::subtitles::Result<engine::subtitles::CaptionAccess> {
        let root = CacheRoot::discover()?.models();
        let (mut cache, _) = ArtifactCache::open(&root, budget, 128)?;
        let providers = Providers::open(
            engine::providers::choose_transcription("whisper.cpp", None)?,
            None,
            &mut cache,
            &Credentials::from_env(),
            &engine::providers::Runtime::default(),
        )?;
        Ok(engine::subtitles::CaptionAccess::new(
            Arc::new(providers),
            Arc::new(DenyUploads),
            Arc::new(std::sync::Mutex::new(cache)),
        )
        .with_clips(Arc::new(shorts::ClipJobs::new(Credentials::from_env()))))
    };
    open().unwrap_or_else(engine::subtitles::CaptionAccess::unavailable)
}

/// The local face detector lent to `reframe.generate` and clip-export layouts. A root that
/// already holds the models cache (through `CaptionAccess::cache`) passes it as `shared`;
/// otherwise the process opens the models cache once here. Either way every load reads
/// through that one cache, and no weights are downloaded.
pub(crate) fn vision_access(
    budget: ByteBudget,
    shared: Option<Arc<std::sync::Mutex<engine::cache::ArtifactCache>>>,
) -> engine::control::VisionAccess {
    shorts::installed_faces(model_lookup(budget, shared))
}

/// The face detector of [`vision_access`] and the local person segmenter lent to
/// `footage.analyze {kind: person_matte}`, both reading through one models-cache lookup; no
/// weights are downloaded.
pub(crate) fn vision_and_segmentation(
    budget: ByteBudget,
    shared: Option<Arc<std::sync::Mutex<engine::cache::ArtifactCache>>>,
) -> (
    engine::control::VisionAccess,
    engine::control::SegmentationAccess,
) {
    let lookup = model_lookup(budget, shared);
    (
        shorts::installed_faces(lookup.clone()),
        shorts::installed_person_matte(lookup),
    )
}

/// The verified models-cache lookup over `shared`, or over the cache this process opens once.
fn model_lookup(
    budget: ByteBudget,
    shared: Option<Arc<std::sync::Mutex<engine::cache::ArtifactCache>>>,
) -> shorts::ModelLookup {
    use ai::foundation::models::Model;
    use engine::cache::ModelLease;
    match shared {
        Some(cache) => Arc::new(
            move |model: &Model,
                  hooks: &dyn ai::ExecutionHooks|
                  -> engine::control::Result<Option<ModelLease>> {
                Ok(cache
                    .lock()
                    .map_err(|_| engine::RuntimeError::Poisoned)?
                    .model(model, hooks)?)
            },
        ),
        None => {
            let opened = engine::models::ModelTransfers::open_native(budget, 128)
                .ok()
                .map(|(transfers, _)| transfers);
            Arc::new(
                move |model: &Model,
                      hooks: &dyn ai::ExecutionHooks|
                      -> engine::control::Result<Option<ModelLease>> {
                    match &opened {
                        Some(transfers) => Ok(transfers.installed(model, hooks)?),
                        None => Err(engine::RuntimeError::CacheBusy.into()),
                    }
                },
            )
        }
    }
}
