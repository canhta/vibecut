//! Freeze-frame request adapter: `timeline.freeze_frame` through control admission.
//!
//! The engine renders the frame, publishes the content-addressed PNG and applies the
//! timeline edit as one undo step. This module only supplies the trusted caller, the
//! runner and the protected paths, and turns a refusal into a typed desktop error.
use crate::{Error, Result};
use engine::{
    ByteBudget, Command, Outcome, Session,
    control::{self, Caller, ControlResult, Policy, Request, ResponseValue},
    jobs::{JobRunner, RunnerConfig, SystemClock},
};
use project::ItemId;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use time::Tick;

/// Distinct idempotency keys across calls in one process.
static REQUESTS: AtomicU64 = AtomicU64::new(0);

/// Trusted composition-root policy: the desktop owns the folder beside the saved project.
struct Allow;

impl Policy for Allow {
    fn authorize(&self, _: &Command, _: &project::Project) -> engine::control::Result<()> {
        Ok(())
    }
}

/// Holds `item` at `at` for `duration`, publishing the still into `dir`.
///
/// A refusal returns [`Error::Invalid`] with the engine's code left out of the
/// message; the session and history are unchanged.
pub fn hold(
    session: &mut Session,
    budget: ByteBudget,
    item: ItemId,
    at: Tick,
    duration: Tick,
    dir: &Path,
    protected: Vec<PathBuf>,
) -> Result<Outcome> {
    session.protect_paths(protected)?;
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 1,
            retained: 1,
            network_attempts: 1,
            backoff: Duration::from_secs(1),
            progress_interval: Duration::from_millis(20),
        },
        budget,
        Arc::new(SystemClock::default()),
    )
    .map_err(Error::from)?;
    let request = Request {
        api_version: control::API_VERSION,
        registry_hash: Some(control::registry_hash().into()),
        session_id: session.session_id().into(),
        request_id: "timeline-freeze-frame".into(),
        idempotency_key: Some(format!(
            "timeline-freeze-frame-{}",
            REQUESTS.fetch_add(1, Ordering::Relaxed)
        )),
        base_rev: Some(session.revision()),
        command: "timeline.freeze_frame".into(),
        params: serde_json::json!({"item": item, "at": at, "duration": duration, "dir": dir}),
    };
    let response = session.control_request(
        request,
        Caller {
            principal: "vibecut-desktop",
            external: false,
            policy: &Allow,
            fonts: None,
            sounds: None,
            music: None,
            stickers: None,
            brand: None,
            media: None,
            chapters: None,
            captions: None,
            vision: None,
        },
        &mut runner,
    )?;
    runner.shutdown();
    match response.value {
        ResponseValue::Result(ControlResult::Edit { outcome }) => Ok(outcome),
        ResponseValue::Result(_) => Err(Error::Invalid("freeze result")),
        ResponseValue::Error(_) => Err(Error::Invalid("freeze refused")),
    }
}
