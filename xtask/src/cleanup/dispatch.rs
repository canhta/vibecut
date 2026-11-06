//! Release settled Orca dispatches and close their terminals after a confirmed receipt.
use super::Perform;
use crate::orca::{self, CommandLine, Worker};
use std::collections::HashSet;

pub(super) fn release_command(environment: Option<&str>, dispatch: &str) -> CommandLine {
    orca::command(
        environment,
        &[
            "orchestration",
            "worker-release",
            "--dispatch",
            dispatch,
            "--json",
            "--full",
        ],
    )
}

/// One observed dispatch and the runtime that owns its record. A federated
/// worker's record lives on the coordinator's local runtime even when its
/// terminal and worktree live on a paired server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ObservedWorker {
    pub(super) worker: Worker,
    pub(super) environment: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReleaseOutcome {
    Released,
    OwnershipTransferred,
    NoOwnedTerminal,
}

pub(super) fn confirm_release(
    receipt: &serde_json::Value,
    dispatch: &str,
) -> Result<ReleaseOutcome, String> {
    let result = receipt
        .get("result")
        .ok_or("worker-release returned no result")?;
    if result.get("dispatchId").and_then(serde_json::Value::as_str) != Some(dispatch) {
        return Err("worker-release returned a different or missing dispatch ID".into());
    }
    match result.get("state").and_then(serde_json::Value::as_str) {
        Some("released" | "already_released") => Ok(ReleaseOutcome::Released),
        Some("retained") => match result.get("reason").and_then(serde_json::Value::as_str) {
            Some("ownership_transferred") => Ok(ReleaseOutcome::OwnershipTransferred),
            Some("no_owned_resource") => Ok(ReleaseOutcome::NoOwnedTerminal),
            reason => Err(format!(
                "worker-release did not confirm release: retained ({reason:?})"
            )),
        },
        state => Err(format!("worker-release did not confirm release: {state:?}")),
    }
}

/// Returns the release receipt for `worker-release`, and `None` for commands
/// that only need their exit status.
pub(super) fn perform(
    command: &CommandLine,
    dry_run: bool,
) -> Result<Option<serde_json::Value>, String> {
    if !dry_run
        && command
            .args
            .get(1)
            .is_some_and(|arg| arg == "worker-release")
    {
        return orca::command_json(command).map(Some);
    }
    orca::perform(command, dry_run).map(|()| None)
}

pub(super) fn terminal_close_command(environment: Option<&str>, terminal: &str) -> CommandLine {
    orca::command(environment, &["terminal", "close", "--terminal", terminal])
}
pub(super) fn ownership_is_released(
    owner: &str,
    workers: &[ObservedWorker],
    targeted: &HashSet<&str>,
) -> bool {
    targeted.contains(owner)
        || workers
            .iter()
            .find(|record| record.worker.dispatch_id == owner)
            .is_some_and(|record| record.worker.already_released())
}

/// Terminal handles belong to the runtime that issued them. A local record can
/// still point at a remote terminal, so close follows the observed host rather
/// than the worktree environment. (The release itself stays local: Orca
/// federates it from the record's owner.)
pub(super) fn terminal_environment(
    record: Option<&ObservedWorker>,
    worker: Option<&Worker>,
    fallback: Option<&str>,
) -> Option<String> {
    match record.and_then(|record| record.environment.as_deref()) {
        Some(environment) => Some(environment.to_owned()),
        None => match worker.and_then(|worker| worker.host.as_deref()) {
            Some("local") | None => None,
            Some(host) => fallback
                .map(str::to_owned)
                .or_else(|| (host != "unknown").then(|| host.to_owned())),
        },
    }
}

pub(super) fn release_dispatch(
    environment: Option<&str>,
    dry_run: bool,
    workers: &[ObservedWorker],
    dispatch: &str,
    targeted: &HashSet<&str>,
    perform: Perform<'_>,
) -> Result<(), String> {
    let record = workers
        .iter()
        .find(|record| record.worker.dispatch_id == dispatch);
    let worker = record.map(|record| &record.worker);
    let terminal = worker.and_then(|worker| worker.terminal.as_deref());
    if terminal.is_none() && worker.is_some_and(Worker::is_settled) {
        println!("[cleanup] {dispatch}: settled with no terminal; cleaned");
        return Ok(());
    }
    if dry_run && terminal.is_none() {
        eprintln!("[cleanup] {dispatch}: no terminal reported; worker-release only");
    }
    // Closing the terminal is dependent on the release succeeding. A refused
    // release can mean the worker started again; never close its terminal in
    // that case. Independent target arguments still continue in `run_with`.
    let release_environment = record.and_then(|record| record.environment.as_deref());
    let receipt = perform(&release_command(release_environment, dispatch), dry_run)
        .map_err(|error| format!("worker-release failed; terminal left open: {error}"))?;
    let outcome = match receipt.as_ref() {
        Some(receipt) => confirm_release(receipt, dispatch)?,
        None => ReleaseOutcome::Released,
    };
    if outcome == ReleaseOutcome::NoOwnedTerminal {
        if terminal.is_some() {
            return Err("worker-release retained a live terminal with no owned resource".into());
        }
        println!("[cleanup] {dispatch}: no owned terminal; cleaned");
        return Ok(());
    }
    if outcome == ReleaseOutcome::OwnershipTransferred {
        let owner = worker
            .and_then(|worker| worker.owner_dispatch_id.as_deref())
            .unwrap_or("another dispatch");
        if !ownership_is_released(owner, workers, targeted) {
            return Err(format!(
                "worker-release left the terminal with {owner}; clean that dispatch too"
            ));
        }
        println!("[cleanup] {dispatch}: terminal owned by {owner}; cleaned");
        return Ok(());
    }
    if let Some(terminal) = terminal {
        let environment = terminal_environment(record, worker, environment);
        perform(
            &terminal_close_command(environment.as_deref(), terminal),
            dry_run,
        )?;
    }
    Ok(())
}
pub(super) fn merge_worker_lists(
    local: Vec<Worker>,
    remote: Vec<Worker>,
    environment: Option<&str>,
) -> Vec<ObservedWorker> {
    let mut local_ids = HashSet::with_capacity(local.len());
    let mut observed = Vec::with_capacity(local.len() + remote.len());
    for worker in local {
        local_ids.insert(worker.dispatch_id.clone());
        observed.push(ObservedWorker {
            worker,
            environment: None,
        });
    }
    for worker in remote {
        if !local_ids.contains(&worker.dispatch_id) {
            observed.push(ObservedWorker {
                worker,
                environment: environment.map(str::to_owned),
            });
        }
    }
    observed
}

/// Dispatch records are owned by the coordinator's local runtime, including
/// federated workers whose terminal is remote. A paired environment is queried
/// as well so dispatches that only exist there are still found.
pub(super) fn discover_workers(environment: Option<&str>) -> Result<Vec<ObservedWorker>, String> {
    let local = orca::workers(None)?;
    match environment {
        Some(environment) => {
            let remote = orca::workers(Some(environment))
                .map_err(|error| format!("`{environment}`: {error}"))?;
            Ok(merge_worker_lists(local, remote, Some(environment)))
        }
        None => Ok(merge_worker_lists(local, Vec::new(), None)),
    }
}
