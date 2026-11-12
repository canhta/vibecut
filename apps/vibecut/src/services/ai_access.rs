//! The desktop's one composition of provider credentials, the models checkpoint cache and
//! registry dispatch for caption and clip jobs. Windows send registry commands through
//! [`run`] with the access an [`Opener`] builds for the run; they never read credentials,
//! open a cache or build `Providers` themselves.
use engine::{
    ByteBudget, Command, Session,
    cache::{ArtifactCache, CacheRoot},
    control::{API_VERSION, Caller, ControlResult, Policy, Request, Revision, registry_hash},
    jobs::JobRunner,
    providers::{ConsentPrompt, Credentials, Providers, Runtime},
    subtitles::CaptionAccess,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

/// The human approval a run's access asks before any remote call.
pub type Consent = Arc<dyn ConsentPrompt + Send + Sync>;

/// Which providers a run resolves. Remote choices are explicit; nothing falls back.
#[derive(Clone, Copy)]
pub enum Choice<'a> {
    /// Speech recognition (and the clip workflow, which shares these providers).
    Transcribe { provider: &'a str, model: &'a str },
    /// Translation only: no speech model or speech credential is needed.
    Translate { provider: &'a str, model: &'a str },
}

/// Builds the access one run lends to the registry. Injected at the lent-access seam:
/// tests return an access over fixture providers, the app opens the real ones.
pub type Opener = Arc<
    dyn for<'a> Fn(Choice<'a>, Consent) -> engine::control::Result<CaptionAccess> + Send + Sync,
>;

/// The shipped opener: credentials from the process environment, local ASR resources, the
/// process models cache opened once, and the clip workflow over the same providers.
pub fn desktop_opener(budget: ByteBudget) -> Opener {
    Arc::new(move |choice, consent| {
        let credentials = Credentials::from_env();
        let cache = models_cache(&budget)?;
        let providers = match choice {
            Choice::Transcribe { provider, model } => {
                let transcription = engine::providers::choose_transcription(provider, Some(model))?;
                let runtime = Runtime {
                    gipformer_runner: engine::providers::local_asr_runner(),
                    ..Default::default()
                };
                let mut guard = cache
                    .lock()
                    .map_err(|_| engine::control::Error::Runtime(engine::RuntimeError::Poisoned))?;
                Providers::open(transcription, None, &mut guard, &credentials, &runtime)?
            }
            Choice::Translate { provider, model } => Providers::translation_only(
                engine::providers::choose_translation(provider, Some(model))?,
                &credentials,
            )?,
        };
        Ok(
            CaptionAccess::new(Arc::new(providers), consent, cache)
                .with_clips(Arc::new(shorts::ClipJobs::new(credentials))),
        )
    })
}

/// A process opens the models cache once; every run locks it only for get/put.
fn models_cache(budget: &ByteBudget) -> engine::control::Result<Arc<Mutex<ArtifactCache>>> {
    static MODELS: Mutex<Option<Arc<Mutex<ArtifactCache>>>> = Mutex::new(None);
    let mut slot = MODELS
        .lock()
        .map_err(|_| engine::control::Error::Runtime(engine::RuntimeError::Poisoned))?;
    if let Some(cache) = slot.as_ref() {
        return Ok(cache.clone());
    }
    let root = CacheRoot::discover()?.models();
    let (cache, _) = ArtifactCache::open(&root, budget.clone(), 128)?;
    let cache = Arc::new(Mutex::new(cache));
    *slot = Some(cache.clone());
    Ok(cache)
}

/// The desktop is the trusted composition root of its own session: consent, not policy,
/// guards remote calls; the one file-publishing command is the user-started `clips.export`.
struct Trusted;
impl Policy for Trusted {
    fn authorize(&self, _: &Command, _: &project::Project) -> engine::control::Result<()> {
        Ok(())
    }
}

/// Run one registry command against the live session through the same admission, receipts
/// and events as any control client. Job commands return `JobAccepted`; the window polls
/// its runner and applies the result with a further command. `base_rev` is the revision the
/// request was computed from (a finished job's accepted revision); `None` takes the current.
pub fn run(
    session: &mut Session,
    runner: &mut JobRunner,
    access: Option<&CaptionAccess>,
    base_rev: Option<Revision>,
    command: &str,
    params: serde_json::Value,
) -> engine::control::Result<ControlResult> {
    static KEYS: AtomicU64 = AtomicU64::new(0);
    let key = format!("desktop-{}", KEYS.fetch_add(1, Ordering::Relaxed));
    let request = Request {
        api_version: API_VERSION,
        registry_hash: Some(registry_hash().into()),
        session_id: session.session_id().into(),
        request_id: key.clone(),
        idempotency_key: Some(key),
        base_rev: Some(base_rev.unwrap_or_else(|| session.revision())),
        command: command.into(),
        params,
    };
    let caller = Caller {
        principal: "vibecut-desktop",
        external: false,
        policy: &Trusted,
        fonts: None,
        sounds: None,
        music: None,
        media: None,
        chapters: None,
        captions: access,
        vision: None,
    };
    session.control_typed(request, caller, runner)
}

/// The accepted job of a job command's result and the revision it was accepted at.
pub fn accepted(
    result: ControlResult,
) -> engine::control::Result<(engine::jobs::JobId, Revision)> {
    match result {
        ControlResult::JobAccepted { job, revision, .. } => Ok((job, revision)),
        _ => Err(engine::control::Error::Edit(engine::EditError::Invalid(
            "command did not start a job",
        ))),
    }
}
