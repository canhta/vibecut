//! Runs one registry job command to completion for a one-shot CLI adapter.
use crate::args::CliError;
use crate::output::wait_for_job;
use engine::control::{
    API_VERSION, Caller, ControlResult, JobView, MediaAccess, Policy, Request, ResponseValue,
    registry_hash,
};
use engine::jobs::JobRunner;
use engine::{Command, EditError, Session};
use serde_json::Value;
use std::sync::Mutex;

/// The adapter is the trusted composition root of its own one-shot session.
struct Trusted;
impl Policy for Trusted {
    fn authorize(&self, _: &Command, _: &project::Project) -> engine::Result<()> {
        Ok(())
    }
}

/// Start `command` through the engine registry like any control client, wait for its
/// job with progress on stderr and return the finished `job_status` view. The job never
/// edits the project; the adapter applies `output` with a typed edit.
pub(crate) fn run_job(
    session: &mut Session,
    runner: &mut JobRunner,
    media: Option<&Mutex<MediaAccess>>,
    command: &str,
    params: Value,
) -> Result<JobView, CliError> {
    let job = match control(session, runner, media, command, params, true)? {
        ControlResult::JobAccepted { job, .. } => job,
        _ => return Err(CliError::Rejected("command did not start a job")),
    };
    wait_for_job(runner, job)?;
    match control(
        session,
        runner,
        media,
        "job_status",
        serde_json::json!({"job": job}),
        false,
    )? {
        ControlResult::Job { snapshot } => Ok(snapshot),
        _ => Err(CliError::Rejected("job status returned no job")),
    }
}

fn control(
    session: &mut Session,
    runner: &mut JobRunner,
    media: Option<&Mutex<MediaAccess>>,
    command: &str,
    params: Value,
    effectful: bool,
) -> Result<ControlResult, CliError> {
    let request = Request {
        api_version: API_VERSION,
        registry_hash: Some(registry_hash().into()),
        session_id: session.session_id().into(),
        request_id: command.into(),
        idempotency_key: effectful.then(|| command.into()),
        base_rev: effectful.then(|| session.revision()),
        command: command.into(),
        params,
    };
    let caller = Caller {
        principal: "vibecut-cli",
        external: false,
        policy: &Trusted,
        fonts: None,
        media,
        chapters: None,
    };
    match session.control_request(request, caller, runner)?.value {
        ResponseValue::Result(result) => Ok(result),
        ResponseValue::Error(code) => Err(CliError::Engine(EditError::Invalid(code_name(code)))),
    }
}

fn code_name(code: engine::control::ErrorCode) -> &'static str {
    use engine::control::ErrorCode as C;
    match code {
        C::UnknownCommand => "unknown command",
        C::InvalidParams => "invalid parameters",
        C::Limit => "limit exceeded",
        C::SchemaMismatch => "schema mismatch",
        C::SessionGone => "session gone",
        C::StaleRevision => "stale revision",
        C::Permission => "permission refused",
        C::IdempotencyConflict => "idempotency conflict",
        C::ReceiptCapacity => "receipt capacity",
        C::ResyncRequired => "resync required",
        C::JobNotRetained => "job output not retained",
        C::Cancelled => "cancelled",
        C::QueueFull => "queue full",
        C::TransactionActive => "transaction active",
        C::EmptyProject => "empty project",
        C::Refused => "refused",
        C::Failed => "failed",
    }
}
