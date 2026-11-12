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

/// The local YuNet detector behind `reframe.generate`. Model files are read from
/// the shared verified cache and never downloaded here: a missing bundle returns
/// the typed setup refusal that names `model download yunet-2026may`. A root that
/// already holds the models cache (through `CaptionAccess::cache`) passes it as
/// `shared`, because that root is opened once per process; otherwise each load
/// opens the cache for the duration of the lookup.
pub(crate) fn vision_access(
    budget: ByteBudget,
    shared: Option<Arc<std::sync::Mutex<engine::cache::ArtifactCache>>>,
) -> engine::control::VisionAccess {
    engine::control::VisionAccess::new(move || {
        let model = engine::models::catalog_model("yunet-2026may")?;
        let hooks = VisionHooks;
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
        .ok_or_else(|| {
            engine::control::Error::Providers(engine::providers::Error::ModelMissing(
                "yunet-2026may",
            ))
        })?;
        let vision = ai::vision::TractVision::load_faces(lease.directory(), 2, 3, &hooks)?;
        let fingerprint = *vision.fingerprint()?.as_bytes();
        Ok((
            Box::new(vision) as Box<dyn ai::vision::VisionDetector>,
            fingerprint,
        ))
    })
}

struct VisionHooks;
impl ai::ExecutionHooks for VisionHooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn stage(&self, _: ai::Stage) {}
}
