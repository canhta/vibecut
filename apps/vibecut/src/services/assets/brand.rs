//! Sends the registry's `brand.*` commands to the lent brand kit store.
use engine::{
    ByteBudget, Command, Session,
    control::{API_VERSION, Caller, ControlResult, Policy, Request, registry_hash},
    jobs::{JobRunner, RunnerConfig, SystemClock},
};
use libraries::brand_kit::BrandKit;
use std::sync::Arc;

/// The desktop is the trusted composition root of its own brand kit requests.
struct Trusted;
impl Policy for Trusted {
    fn authorize(&self, _: &Command, _: &project::Project) -> engine::control::Result<()> {
        Ok(())
    }
}

/// Run one `brand.*` registry command over `store`, the way the music commands run: an empty
/// workflow session per call and the store as the lent access. A listing hashes every kit file,
/// so callers run this off the UI thread. Callable from any thread.
pub fn run_brand(
    store: &BrandKit,
    budget: &ByteBudget,
    command: &str,
    params: serde_json::Value,
) -> engine::control::Result<ControlResult> {
    let mut session = Session::new(
        project::Project::empty(project::ProjectId(0), String::new().into())?,
        1,
    )
    .map_err(engine::control::Error::Edit)?;
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
    let result = store.with_control(|access| {
        session.control_typed(
            request,
            Caller {
                principal: "vibecut-desktop",
                external: false,
                policy: &Trusted,
                fonts: None,
                sounds: None,
                music: None,
                stickers: None,
                brand: Some(access),
                media: None,
                chapters: None,
                captions: None,
                vision: None,
            },
            &mut runner,
        )
    });
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
