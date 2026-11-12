//! Runs one registry job command to completion for a one-shot CLI adapter.
use super::registry::{self, Lent};
use crate::args::CliError;
use crate::output::wait_for_job;
use engine::Session;
use engine::control::{ControlResult, JobView, MediaAccess};
use engine::jobs::JobRunner;
use serde_json::Value;
use std::sync::Mutex;

/// Start `command` through the engine registry like any control client, wait for its
/// job with progress on stderr and return the finished `job_status` view. The job never
/// edits the project; the adapter applies `output` with a typed edit.
pub(crate) fn run_job(
    session: &mut Session,
    runner: &mut JobRunner,
    media: Option<&Mutex<MediaAccess>>,
    vision: Option<&engine::control::VisionAccess>,
    command: &str,
    params: Value,
) -> Result<JobView, CliError> {
    run_job_with(
        session,
        runner,
        Lent {
            media,
            vision,
            ..Lent::default()
        },
        command,
        params,
    )
}

/// `run_job` with every access object the composition root lends to the request.
pub(crate) fn run_job_with(
    session: &mut Session,
    runner: &mut JobRunner,
    lent: Lent<'_>,
    command: &str,
    params: Value,
) -> Result<JobView, CliError> {
    let job = match registry::run(session, runner, lent, command, params, true)? {
        ControlResult::JobAccepted {
            job,
            word_highlight_warnings,
            ..
        } => {
            if (!word_highlight_warnings.warnings.is_empty()
                || word_highlight_warnings.additional > 0)
                && let Ok(value) = serde_json::to_string(&word_highlight_warnings)
            {
                eprintln!("{value}");
            }
            job
        }
        _ => return Err(CliError::Rejected("command did not start a job")),
    };
    wait_for_job(runner, job)?;
    match registry::run(
        session,
        runner,
        lent,
        "job_status",
        serde_json::json!({"job": job}),
        false,
    )? {
        ControlResult::Job { snapshot } => Ok(snapshot),
        _ => Err(CliError::Rejected("job status returned no job")),
    }
}
