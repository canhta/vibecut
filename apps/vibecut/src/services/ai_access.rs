//! The desktop's one composition of provider credentials, the models checkpoint cache and
//! registry dispatch for caption and clip jobs. Windows send registry commands through
//! [`run`] with the access an [`Opener`] builds for the run; they never read credentials,
//! open a cache or build `Providers` themselves.
use engine::{
    ByteBudget, Command, Session,
    cache::{ArtifactCache, CacheRoot},
    control::{API_VERSION, Caller, ControlResult, Policy, Request, Revision, registry_hash},
    jobs::JobRunner,
    providers::{ConsentPrompt, Credentials, Providers, Runtime, TextAccess, UploadRequest},
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

/// Builds the text-provider access one text run lends. Its consent is the run's own sheet,
/// handed in by the caller, so no second consent path exists.
pub type TextOpener = Arc<
    dyn Fn(TextTask, &str, &str, Consent) -> engine::providers::Result<TextAccess> + Send + Sync,
>;

/// The text task a lent text access serves; each names its own provider catalog entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextTask {
    Chapters,
    Fillers,
}

/// The shipped text opener: the configured provider and model through the task's chooser, with
/// credentials from the process environment. Nothing falls back to another provider.
pub fn desktop_text_opener() -> TextOpener {
    Arc::new(|task, provider, model, consent| {
        let provider = match task {
            TextTask::Chapters => engine::providers::choose_chapter(provider, Some(model))?,
            TextTask::Fillers => engine::providers::choose_filler(provider, Some(model))?,
        };
        TextAccess::open(
            provider,
            &Credentials::from_env(),
            Box::new(SharedConsent(consent)),
        )
    })
}

/// Lends one run's sheet to a lent access, which owns its consent box.
pub struct SharedConsent(pub Consent);
impl ConsentPrompt for SharedConsent {
    fn approve(&self, request: &UploadRequest) -> engine::providers::Result<()> {
        self.0.approve(request)
    }
    fn approve_unknown_price(&self, request: &UploadRequest) -> engine::providers::Result<()> {
        self.0.approve_unknown_price(request)
    }
}

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
        Ok(CaptionAccess::new(Arc::new(providers), consent, cache)
            .with_clips(Arc::new(shorts::ClipJobs::new(credentials))))
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
    execute(session, runner, access, None, base_rev, command, params)
}

/// Run one text-provider registry command (`chapters.generate`) with the text access the
/// caller lent for it. The job asks that access's own consent sheet before any upload.
pub fn run_text(
    session: &mut Session,
    runner: &mut JobRunner,
    text: &TextAccess,
    command: &str,
    params: serde_json::Value,
) -> engine::control::Result<ControlResult> {
    execute(session, runner, None, Some(text), None, command, params)
}

fn execute(
    session: &mut Session,
    runner: &mut JobRunner,
    access: Option<&CaptionAccess>,
    text: Option<&TextAccess>,
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
        stickers: None,
        brand: None,
        media: None,
        chapters: text,
        captions: access,
        vision: None,
    };
    session.control_typed(request, caller, runner)
}

/// The accepted job of a job command's result and the revision it was accepted at.
pub fn accepted(result: ControlResult) -> engine::control::Result<(engine::jobs::JobId, Revision)> {
    match result {
        ControlResult::JobAccepted { job, revision, .. } => Ok((job, revision)),
        _ => Err(engine::control::Error::Edit(engine::EditError::Invalid(
            "command did not start a job",
        ))),
    }
}
