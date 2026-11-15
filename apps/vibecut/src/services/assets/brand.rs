//! Sends the registry's `brand.*` commands to the lent brand kit store.
use engine::{
    ByteBudget, Command, Session,
    control::{API_VERSION, Caller, ControlResult, Policy, Request, registry_hash},
    jobs::{JobRunner, RunnerConfig, SystemClock},
};
use libraries::brand_kit::BrandKit;
use std::{path::PathBuf, sync::Arc};

/// The desktop is the trusted composition root of its own brand kit requests.
struct Trusted;
impl Policy for Trusted {
    fn authorize(&self, _: &Command, _: &project::Project) -> engine::control::Result<()> {
        Ok(())
    }
}

/// Run one `brand.*` registry command over `store`, the way the music commands run: an empty
/// workflow session per call and the store as the lent access. A listing hashes every kit file,
/// so callers run this off the UI thread. Callable from any thread. `brand.copy_media` goes
/// through [`copy_media`], which gives the session its media folder.
pub fn run_brand(
    store: &BrandKit,
    budget: &ByteBudget,
    command: &str,
    params: serde_json::Value,
) -> engine::control::Result<ControlResult> {
    run(store, budget, None, command, params)
}

/// Copy one active-kit file into `media_dir`, the project's media folder
/// (`project::media_dir` of its file), through `brand.copy_media`; returns the copy's path.
/// The folder is the session's, so the command names none. Reads a whole file: run it off the
/// UI thread.
pub fn copy_media(
    store: &BrandKit,
    budget: &ByteBudget,
    media_dir: PathBuf,
    media: u64,
) -> engine::control::Result<PathBuf> {
    match run(
        store,
        budget,
        Some(media_dir),
        "brand.copy_media",
        serde_json::json!({"media": media}),
    )? {
        ControlResult::BrandMediaCopied { path } => Ok(path),
        _ => Err(engine::control::Error::Edit(engine::EditError::Invalid(
            "brand kit copy result",
        ))),
    }
}

fn run(
    store: &BrandKit,
    budget: &ByteBudget,
    media_dir: Option<PathBuf>,
    command: &str,
    params: serde_json::Value,
) -> engine::control::Result<ControlResult> {
    let mut session = Session::new(
        project::Project::empty(project::ProjectId(0), String::new().into())?,
        1,
    )
    .map_err(engine::control::Error::Edit)?;
    if let Some(dir) = media_dir {
        session
            .set_media_dir(dir)
            .map_err(engine::control::Error::Edit)?;
    }
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 1,
            retained: 1,
            network_attempts: 1,
            backoff: std::time::Duration::from_millis(250),
            progress_interval: std::time::Duration::from_millis(50),
        },
        budget.clone(),
        Arc::new(SystemClock::default()),
    )?;
    let request = Request {
        api_version: API_VERSION,
        registry_hash: Some(registry_hash().into()),
        session_id: session.session_id().into(),
        request_id: command.into(),
        idempotency_key: None,
        base_rev: None,
        command: command.into(),
        params,
    };
    let result = session.control_typed(
        request,
        Caller {
            principal: "vibecut-desktop",
            external: false,
            policy: &Trusted,
            fonts: None,
            sounds: None,
            music: None,
            stickers: None,
            brand: Some(store),
            media: None,
            chapters: None,
            captions: None,
            vision: None,
        },
        &mut runner,
    );
    runner.shutdown();
    result
}

/// The active kit a `brand.list` or edit answered with.
pub fn kit(
    result: ControlResult,
) -> engine::control::Result<Option<engine::control::BrandKitView>> {
    match result {
        ControlResult::BrandKit { kit } => Ok(kit),
        _ => Err(engine::control::Error::Edit(engine::EditError::Invalid(
            "brand kit result",
        ))),
    }
}

/// The active kit's font families in kit order, read through `brand.list`; none without an
/// active kit. The listing hashes every kit file: run it off the UI thread.
pub fn fonts(store: &BrandKit, budget: &ByteBudget) -> engine::control::Result<Vec<String>> {
    let kit = kit(run_brand(
        store,
        budget,
        "brand.list",
        serde_json::json!({}),
    )?)?;
    Ok(kit.map(|kit| kit.fonts).unwrap_or_default())
}
