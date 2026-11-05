//! Release settled Orca workers, remove ticket worktrees and prune build caches.

use crate::orca::{self, CommandLine, Worker};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Dispatch(String),
    Worktree(String),
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    environment: Option<String>,
    dry_run: bool,
    targets: Vec<Target>,
}

fn target(value: &str) -> Result<Target, String> {
    if let Some(dispatch) = value.strip_prefix("ctx_") {
        if dispatch.is_empty()
            || !dispatch
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        {
            return Err(format!("invalid dispatch id `{value}`"));
        }
        return Ok(Target::Dispatch(value.to_owned()));
    }
    orca::worktree_name(value)?;
    Ok(Target::Worktree(value.to_owned()))
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dry-run" => options.dry_run = true,
            "--environment" => {
                if options.environment.is_some() {
                    return Err("--environment may be given only once".into());
                }
                let value = args
                    .next()
                    .ok_or("--environment requires a saved environment name")?;
                if value.is_empty() || value.starts_with('-') {
                    return Err(format!("invalid environment `{value}`"));
                }
                options.environment = Some(value.clone());
            }
            "--" => {
                for value in args.by_ref() {
                    options.targets.push(target(value)?);
                }
            }
            value if value.starts_with("--") => {
                return Err(format!("unknown option `{value}`"));
            }
            value => options.targets.push(target(value)?),
        }
    }
    if options.targets.is_empty() {
        return Err(
            "usage: cargo xtask cleanup [--environment NAME] [--dry-run] <ctx_dispatch|worktree-name>..."
                .into(),
        );
    }
    Ok(options)
}

fn release_command(environment: Option<&str>, dispatch: &str) -> CommandLine {
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
struct ObservedWorker {
    worker: Worker,
    environment: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReleaseOutcome {
    Released,
    OwnershipTransferred,
    NoOwnedTerminal,
}

fn confirm_release(receipt: &serde_json::Value, dispatch: &str) -> Result<ReleaseOutcome, String> {
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
fn perform(command: &CommandLine, dry_run: bool) -> Result<Option<serde_json::Value>, String> {
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

fn terminal_close_command(environment: Option<&str>, terminal: &str) -> CommandLine {
    orca::command(environment, &["terminal", "close", "--terminal", terminal])
}

fn worktree_remove_command(environment: Option<&str>, path: &Path) -> CommandLine {
    let selector = format!("path:{}", path.display());
    orca::command(
        environment,
        &["worktree", "rm", "--worktree", &selector, "--delete-branch"],
    )
}

type Perform<'a> =
    &'a mut dyn FnMut(&CommandLine, bool) -> Result<Option<serde_json::Value>, String>;

fn ownership_is_released(
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
fn terminal_environment(
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

fn release_dispatch(
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

fn remove_scratch(path: &Path, dry_run: bool) -> Result<(), String> {
    let scratch = path.join(".scratch");
    if !scratch.exists() {
        return Ok(());
    }
    if dry_run {
        println!("remove {}", scratch.display());
        return Ok(());
    }
    fs::remove_dir_all(&scratch).map_err(|error| format!("{}: {error}", scratch.display()))
}

fn resolve_worktree(
    worktrees: &[orca::Worktree],
    workspaces: Option<&Path>,
    name: &str,
) -> Result<(PathBuf, Option<String>), String> {
    if let Some(worktree) = worktrees.iter().find(|worktree| {
        worktree.name == name
            || worktree
                .path
                .file_name()
                .and_then(|file| file.to_str())
                .is_some_and(|file| file == name)
    }) {
        if worktree.is_main {
            return Err(format!("refusing to remove the main checkout `{name}`"));
        }
        return Ok((worktree.path.clone(), worktree.branch.clone()));
    }
    let path = workspaces
        .map(|directory| directory.join(name))
        .ok_or_else(|| {
            format!("cannot resolve `{name}`: no Orca worktree or workspaces directory found")
        })?;
    Ok((path, None))
}

fn git_command(main: &Path, args: &[&str]) -> CommandLine {
    let mut arguments = vec!["-C".to_owned(), main.to_string_lossy().into_owned()];
    arguments.extend(args.iter().map(|arg| (*arg).to_owned()));
    CommandLine::new("git", arguments)
}

fn git_output(main: &Path, args: &[&str]) -> Result<Output, String> {
    git_command(main, args).run()
}

/// Branch names come from Orca data, but they still reach Git as option-like
/// arguments. Reject anything that is not a plain local branch ref before use.
fn valid_branch_name(branch: &str) -> bool {
    !branch.is_empty()
        && branch != "HEAD"
        && branch != "main"
        && branch != "master"
        && !branch.starts_with('-')
        && !branch.starts_with('/')
        && !branch.ends_with('/')
        && !branch.ends_with('.')
        && !branch.ends_with(".lock")
        && !branch.contains("..")
        && !branch.contains("@{")
        && !branch.contains("//")
        && !branch.chars().any(|character| {
            character.is_control()
                || character.is_whitespace()
                || matches!(character, '\\' | '~' | '^' | ':' | '?' | '*' | '[')
        })
}

/// `git cherry` marks a patch already upstream with `-` and a unique patch
/// with `+`. An empty listing means the branch is already contained. Anything
/// malformed is retained rather than guessed at.
fn cherry_has_no_unique_patches(output: &str) -> bool {
    output.lines().all(|line| {
        let line = line.trim_end();
        line.is_empty() || line.starts_with('-')
    })
}

fn branch_is_merged(main: &Path, branch: &str) -> Result<bool, String> {
    let local = format!("refs/heads/{branch}");
    let remote = format!("refs/remotes/origin/{branch}");
    let reference = if git_output(main, &["rev-parse", "--verify", "--quiet", &local])?
        .status
        .success()
    {
        local
    } else if git_output(main, &["rev-parse", "--verify", "--quiet", &remote])?
        .status
        .success()
    {
        remote
    } else {
        return Ok(false);
    };
    if git_output(
        main,
        &["merge-base", "--is-ancestor", &reference, "origin/main"],
    )?
    .status
    .success()
    {
        return Ok(true);
    }
    let cherry = git_output(main, &["cherry", "origin/main", &reference])?;
    if !cherry.status.success() {
        return Err(format!(
            "git cherry failed: {}",
            String::from_utf8_lossy(&cherry.stderr).trim()
        ));
    }
    Ok(cherry_has_no_unique_patches(&String::from_utf8_lossy(
        &cherry.stdout,
    )))
}

fn remote_branch_delete_command(main: &Path, branch: &str) -> CommandLine {
    git_command(main, &["push", "origin", "--delete", branch])
}

fn local_branch_delete_command(main: &Path, branch: &str) -> CommandLine {
    git_command(main, &["branch", "-D", branch])
}

fn delete_remote_merged_branch(
    main: &Path,
    dry_run: bool,
    branch: &str,
    perform: Perform<'_>,
) -> Result<(), String> {
    // Deleting the remote name is a no-op if somebody already removed it.
    let remote = git_output(
        main,
        &["ls-remote", "--exit-code", "--heads", "origin", branch],
    )?;
    if remote.status.success() {
        perform(&remote_branch_delete_command(main, branch), dry_run)?;
    } else if remote.status.code() != Some(2) {
        return Err(format!(
            "git ls-remote failed: {}",
            String::from_utf8_lossy(&remote.stderr).trim()
        ));
    }
    Ok(())
}

fn delete_local_merged_branch(
    main: &Path,
    dry_run: bool,
    branch: &str,
    perform: Perform<'_>,
) -> Result<(), String> {
    let local = git_output(
        main,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )?;
    if local.status.success() {
        perform(&local_branch_delete_command(main, branch), dry_run)?;
    }
    Ok(())
}

fn remove_worktree(
    main: &Path,
    environment: Option<&str>,
    dry_run: bool,
    worktrees: &[orca::Worktree],
    workspaces: Option<&Path>,
    name: &str,
    perform: Perform<'_>,
) -> Result<(), String> {
    let (path, branch) = resolve_worktree(worktrees, workspaces, name)?;
    // Resolve branch safety while the checkout still exists; `worktree rm`
    // may delete the local ref it was checked out on. Branch cleanup is also
    // attempted before removal so a remote failure can be retried with the
    // worktree still present.
    let merged = match branch.as_deref() {
        Some(branch) if valid_branch_name(branch) => Some(branch_is_merged(main, branch)?),
        Some(branch) => return Err(format!("invalid branch name `{branch}`")),
        None => None,
    };
    if let Some(branch) = branch.as_deref() {
        if merged == Some(true) {
            delete_remote_merged_branch(main, dry_run, branch, perform)?;
        } else {
            println!("[cleanup] branch {branch}: unique patches; retained");
        }
    }
    // A paired server owns the remote directory; `worktree rm` deletes its
    // ignored `.scratch` with the checkout. Locally the ignored tree goes first
    // so Git never has to force over untracked report files.
    if environment.is_none() {
        remove_scratch(&path, dry_run)?;
    }
    perform(&worktree_remove_command(environment, &path), dry_run)?;
    if let (Some(branch), Some(true)) = (branch, merged) {
        delete_local_merged_branch(main, dry_run, &branch, perform)?;
    }
    Ok(())
}

fn pull(main: &Path, dry_run: bool) -> Result<(), String> {
    let command = CommandLine::new(
        "git",
        [
            "-C".to_owned(),
            main.to_string_lossy().into_owned(),
            "pull".to_owned(),
            "--ff-only".to_owned(),
        ],
    );
    orca::perform(&command, dry_run)
}

fn prune(dry_run: bool) -> Result<(), String> {
    crate::build::prune_cache(dry_run)
}

struct Discovery {
    worktrees: Result<Vec<orca::Worktree>, String>,
    workers: Result<Vec<ObservedWorker>, String>,
}

fn run_with<F, P, L>(
    main: &Path,
    options: &Options,
    discovery: Discovery,
    mut perform: F,
    prune: P,
    pull: L,
) -> Result<(), String>
where
    F: FnMut(&CommandLine, bool) -> Result<Option<serde_json::Value>, String>,
    P: FnOnce(bool) -> Result<(), String>,
    L: FnOnce(&Path, bool) -> Result<(), String>,
{
    let environment = options.environment.as_deref();
    let mut failures = 0usize;
    // Discovery is a cleanup step in its own right: an incomplete view must
    // never turn into a successful exit just because later commands ran.
    let worktrees = match discovery.worktrees {
        Ok(worktrees) => worktrees,
        Err(error) => {
            eprintln!("[cleanup] worktree list: {error}");
            failures += 1;
            Vec::new()
        }
    };
    let workspaces = orca::workspaces_dir(&worktrees);
    let needs_workers = options
        .targets
        .iter()
        .any(|target| matches!(target, Target::Dispatch(_)));
    let workers = if needs_workers {
        match discovery.workers {
            Ok(workers) => workers,
            Err(error) => {
                eprintln!("[cleanup] worker list: {error}");
                failures += 1;
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    // Refresh `origin/main` before branch cleanup so a merge pushed by the
    // reviewer is visible to ancestry and `git cherry` checks.
    if let Err(error) = pull(main, options.dry_run) {
        eprintln!("[cleanup] pull: {error}");
        failures += 1;
    }
    let targeted: HashSet<&str> = options
        .targets
        .iter()
        .filter_map(|target| match target {
            Target::Dispatch(dispatch) => Some(dispatch.as_str()),
            Target::Worktree(_) => None,
        })
        .collect();
    for target in &options.targets {
        let result = match target {
            Target::Dispatch(dispatch) => release_dispatch(
                environment,
                options.dry_run,
                &workers,
                dispatch,
                &targeted,
                &mut perform,
            ),
            Target::Worktree(name) => remove_worktree(
                main,
                environment,
                options.dry_run,
                &worktrees,
                workspaces.as_deref(),
                name,
                &mut perform,
            ),
        };
        if let Err(error) = result {
            eprintln!("[cleanup] {target:?}: {error}");
            failures += 1;
        }
    }
    if let Err(error) = prune(options.dry_run) {
        eprintln!("[cleanup] build cache: {error}");
        failures += 1;
    }
    if failures == 0 {
        Ok(())
    } else {
        Err(format!("{failures} cleanup step(s) failed"))
    }
}

fn merge_worker_lists(
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
fn discover_workers(environment: Option<&str>) -> Result<Vec<ObservedWorker>, String> {
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

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let options = parse(args)?;
    let environment = options.environment.as_deref();
    let main = crate::checkout::main(root)?;
    let worktrees = orca::worktrees(environment);
    let needs_workers = options
        .targets
        .iter()
        .any(|target| matches!(target, Target::Dispatch(_)));
    let workers = if needs_workers {
        discover_workers(environment)
    } else {
        Ok(Vec::new())
    };
    run_with(
        &main,
        &options,
        Discovery { worktrees, workers },
        perform,
        prune,
        pull,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_classify_dispatches_and_worktrees_and_reject_hostile_paths() {
        for (input, expected) in [
            ("ctx_abc-123", Ok(Target::Dispatch("ctx_abc-123".into()))),
            (
                "opencode-core-58",
                Ok(Target::Worktree("opencode-core-58".into())),
            ),
            ("ctx_", Err("invalid dispatch id")),
            ("ctx_../x", Err("invalid dispatch id")),
            ("../escape", Err("invalid worktree name")),
            ("a/b", Err("invalid worktree name")),
            ("a\\b", Err("invalid worktree name")),
            ("C:worktree", Err("invalid worktree name")),
            (".", Err("invalid worktree name")),
            ("", Err("invalid worktree name")),
        ] {
            match (target(input), expected) {
                (Ok(actual), Ok(expected)) => assert_eq!(actual, expected),
                (Err(actual), Err(expected)) => assert!(
                    actual.contains(expected),
                    "{input}: expected `{expected}` in `{actual}`"
                ),
                (actual, expected) => panic!("{input}: {actual:?} != {expected:?}"),
            }
        }
    }

    #[test]
    fn arguments_accept_any_option_order_and_require_at_least_one_target() {
        let args: Vec<String> = ["ticket-58", "--dry-run", "--environment", "Window"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(
            parse(&args).unwrap(),
            Options {
                environment: Some("Window".into()),
                dry_run: true,
                targets: vec![Target::Worktree("ticket-58".into())],
            }
        );
        assert!(parse(&[]).unwrap_err().contains("usage:"));
        assert!(
            parse(&["--environment".into(), "Window".into()])
                .unwrap_err()
                .contains("usage:")
        );
        assert!(
            parse(&["--environment".into()])
                .unwrap_err()
                .contains("requires")
        );
        assert!(
            parse(&[
                "--environment".into(),
                "A".into(),
                "--environment".into(),
                "B".into()
            ])
            .unwrap_err()
            .contains("only once")
        );
        assert!(
            parse(&["--unknown".into()])
                .unwrap_err()
                .contains("unknown")
        );
    }

    fn worker(dispatch: &str, state: &str, terminal: Option<&str>) -> Worker {
        Worker {
            dispatch_id: dispatch.into(),
            state: Some(state.into()),
            terminal: terminal.map(str::to_owned),
            terminal_state: None,
            host: Some("local".into()),
            owner_dispatch_id: None,
            release_state: None,
        }
    }

    fn observed(worker: Worker) -> ObservedWorker {
        ObservedWorker {
            worker,
            environment: None,
        }
    }

    fn reported_worker() -> Vec<ObservedWorker> {
        vec![observed(worker("ctx_abc", "succeeded", Some("term_x")))]
    }

    fn targeted<'a>(values: &'a [&'a str]) -> HashSet<&'a str> {
        values.iter().copied().collect()
    }

    #[test]
    fn successful_release_then_closes_the_reported_terminal() {
        let workers = reported_worker();
        let mut commands = Vec::new();
        release_dispatch(
            Some("Window"),
            false,
            &workers,
            "ctx_abc",
            &targeted(&["ctx_abc"]),
            &mut |command, _| {
                commands.push(command.render());
                Ok(None)
            },
        )
        .unwrap();
        // The dispatch record is local, so the release is local even when the
        // worktree argument names a paired environment.
        assert_eq!(
            commands,
            [
                "orca orchestration worker-release --dispatch ctx_abc --json --full",
                "orca terminal close --terminal term_x",
            ]
        );
    }

    #[test]
    fn remote_terminal_close_uses_the_paired_environment() {
        let mut worker = worker("ctx_abc", "succeeded", Some("term_x"));
        worker.host = Some("Window".into());
        let workers = vec![observed(worker)];
        let mut commands = Vec::new();
        release_dispatch(
            Some("Window"),
            false,
            &workers,
            "ctx_abc",
            &targeted(&["ctx_abc"]),
            &mut |command, _| {
                commands.push(command.render());
                Ok(None)
            },
        )
        .unwrap();
        assert_eq!(
            commands,
            [
                "orca orchestration worker-release --dispatch ctx_abc --json --full",
                "orca terminal close --terminal term_x --environment Window",
            ]
        );
    }

    #[test]
    fn settled_dispatch_without_terminal_is_cleaned_without_commands() {
        let workers = vec![observed(worker("ctx_abc", "succeeded", None))];
        let mut commands = Vec::new();
        release_dispatch(
            Some("Window"),
            false,
            &workers,
            "ctx_abc",
            &targeted(&["ctx_abc"]),
            &mut |command, _| {
                commands.push(command.render());
                Ok(None)
            },
        )
        .unwrap();
        assert!(commands.is_empty(), "{commands:?}");
    }

    #[test]
    fn live_terminal_release_refused_skips_close_and_fails() {
        let workers = reported_worker();
        let mut commands = Vec::new();
        let error = release_dispatch(
            None,
            false,
            &workers,
            "ctx_abc",
            &targeted(&["ctx_abc"]),
            &mut |command, _| {
                commands.push(command.render());
                if command.args.iter().any(|arg| arg == "worker-release") {
                    Err("refused: worker still running".into())
                } else {
                    Ok(None)
                }
            },
        )
        .unwrap_err();
        assert!(error.contains("refused"), "{error}");
        assert_eq!(
            commands,
            ["orca orchestration worker-release --dispatch ctx_abc --json --full"]
        );
    }

    #[test]
    fn only_confirmed_release_receipts_allow_terminal_closure() {
        for (state, dispatch, allowed) in [
            ("released", "ctx_abc", true),
            ("already_released", "ctx_abc", true),
            ("retained", "ctx_abc", false),
            ("release_pending", "ctx_abc", false),
            ("release_unknown", "ctx_abc", false),
            ("unexpected", "ctx_abc", false),
            ("released", "ctx_other", false),
        ] {
            let receipt = serde_json::json!({"result": {"state": state, "dispatchId": dispatch}});
            let mut commands = Vec::new();
            let result = release_dispatch(
                None,
                false,
                &reported_worker(),
                "ctx_abc",
                &targeted(&["ctx_abc"]),
                &mut |command, _| {
                    commands.push(command.render());
                    if command
                        .args
                        .get(1)
                        .is_some_and(|arg| arg == "worker-release")
                    {
                        Ok(Some(receipt.clone()))
                    } else {
                        Ok(None)
                    }
                },
            );
            assert_eq!(result.is_ok(), allowed, "{state} / {dispatch}");
            assert_eq!(
                commands.len(),
                if allowed { 2 } else { 1 },
                "{state} / {dispatch}"
            );
        }
        assert!(confirm_release(&serde_json::json!({}), "ctx_abc").is_err());
        assert!(
            confirm_release(
                &serde_json::json!({"result": {"dispatchId": "ctx_abc"}}),
                "ctx_abc"
            )
            .is_err()
        );
    }

    #[test]
    fn no_owned_terminal_is_cleaned_only_without_a_reported_live_terminal() {
        for (terminal, allowed) in [(None, true), (Some("term_x"), false)] {
            let workers = vec![observed(worker("ctx_abc", "succeeded", terminal))];
            let receipt = serde_json::json!({
                "result": {
                    "state": "retained",
                    "reason": "no_owned_resource",
                    "dispatchId": "ctx_abc"
                }
            });
            let mut commands = Vec::new();
            let result = release_dispatch(
                None,
                false,
                &workers,
                "ctx_abc",
                &targeted(&["ctx_abc"]),
                &mut |command, _| {
                    commands.push(command.render());
                    if command
                        .args
                        .get(1)
                        .is_some_and(|arg| arg == "worker-release")
                    {
                        Ok(Some(receipt.clone()))
                    } else {
                        Ok(None)
                    }
                },
            );
            assert_eq!(result.is_ok(), allowed, "{terminal:?}");
            // A settled dispatch with no reported terminal never reaches the
            // release call; a live one is refused rather than silently closed.
            assert_eq!(
                commands.len(),
                usize::from(terminal.is_some()),
                "{terminal:?}: {commands:?}"
            );
        }
    }

    #[test]
    fn ownership_transferred_is_cleaned_when_the_owner_is_targeted_or_released() {
        for (owner_released, include_owner, allowed) in [
            (false, true, true),
            (true, false, true),
            (false, false, false),
        ] {
            let mut ab = worker("ctx_abc", "succeeded", Some("term_x"));
            ab.owner_dispatch_id = Some("ctx_owner".into());
            let mut owner = worker("ctx_owner", "succeeded", Some("term_x"));
            owner.terminal_state = owner_released.then(|| "released".into());
            let workers = vec![observed(ab), observed(owner)];
            let targets = if include_owner {
                targeted(&["ctx_abc", "ctx_owner"])
            } else {
                targeted(&["ctx_abc"])
            };
            let receipt = serde_json::json!({
                "result": {
                    "state": "retained",
                    "reason": "ownership_transferred",
                    "dispatchId": "ctx_abc"
                }
            });
            let mut commands = Vec::new();
            let result = release_dispatch(
                None,
                false,
                &workers,
                "ctx_abc",
                &targets,
                &mut |command, _| {
                    commands.push(command.render());
                    if command
                        .args
                        .get(1)
                        .is_some_and(|arg| arg == "worker-release")
                    {
                        Ok(Some(receipt.clone()))
                    } else {
                        Ok(None)
                    }
                },
            );
            assert_eq!(result.is_ok(), allowed, "owner released={owner_released}");
            // The follow-up dispatch owns the terminal, so cleanup never
            // closes it from the former owner.
            assert_eq!(commands.len(), 1, "owner released={owner_released}");
        }
    }

    #[test]
    fn worker_discovery_prefers_the_local_record_and_tags_remote_only_dispatches() {
        let local = vec![worker("ctx_local", "succeeded", None)];
        let remote = vec![
            worker("ctx_local", "failed", Some("term_remote")),
            worker("ctx_remote", "succeeded", Some("term_remote")),
        ];
        let merged = merge_worker_lists(local, remote, Some("Window"));
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].worker.dispatch_id, "ctx_local");
        assert_eq!(merged[0].environment, None);
        assert_eq!(merged[0].worker.state.as_deref(), Some("succeeded"));
        assert_eq!(merged[1].worker.dispatch_id, "ctx_remote");
        assert_eq!(merged[1].environment.as_deref(), Some("Window"));
    }

    #[test]
    fn discovery_failures_keep_the_exit_code_non_zero_after_targets_continue() {
        let options = Options {
            environment: None,
            dry_run: true,
            targets: vec![Target::Dispatch("ctx_ok".into())],
        };
        let mut performed = 0usize;
        let error = run_with(
            Path::new("/main"),
            &options,
            Discovery {
                worktrees: Err("worktree list down".into()),
                workers: Err("worker list down".into()),
            },
            |_, _| {
                performed += 1;
                Ok(None)
            },
            |_| Ok(()),
            |_, _| Ok(()),
        )
        .unwrap_err();
        assert_eq!(performed, 1, "an independent dispatch target was skipped");
        assert!(error.contains("2 cleanup step(s) failed"), "{error}");
    }

    #[test]
    fn worktree_removal_targets_the_resolved_path_and_deletes_the_branch() {
        assert_eq!(
            worktree_remove_command(Some("Window"), Path::new("/workspaces/repo/ticket-58"))
                .render(),
            "orca worktree rm --worktree path:/workspaces/repo/ticket-58 --delete-branch --environment Window"
        );
        assert_eq!(
            remote_branch_delete_command(Path::new("/repo"), "fix/p0-core-63-cleanup").render(),
            "git -C /repo push origin --delete fix/p0-core-63-cleanup"
        );
        assert_eq!(
            local_branch_delete_command(Path::new("/repo"), "fix/p0-core-63-cleanup").render(),
            "git -C /repo branch -D fix/p0-core-63-cleanup"
        );
    }

    #[test]
    fn branch_names_reject_option_like_and_hostile_refs() {
        for valid in [
            "fix/p0-core-63-cleanup-settled-dispatch",
            "feat/p1-parity-91-projects-page-ui",
            "ticket-58",
        ] {
            assert!(valid_branch_name(valid), "{valid}");
        }
        for hostile in [
            "",
            "HEAD",
            "main",
            "master",
            "-D",
            "--force",
            "/absolute",
            "trailing/",
            "double//slash",
            "range..dots",
            "at@{brace",
            "back\\slash",
            "caret^",
            "colon:name",
            "question?",
            "star*",
            "bracket[",
            "space name",
            "control\u{7}",
            "trailing.",
            "branch.lock",
        ] {
            assert!(!valid_branch_name(hostile), "{hostile:?}");
        }
    }

    struct GitDirectory(PathBuf);

    impl GitDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "vibecut-cleanup-git-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            let directory = Self(path);
            directory.git(&["init", "-q"]);
            directory.git(&["config", "user.email", "test@example.com"]);
            directory.git(&["config", "user.name", "Test"]);
            fs::write(directory.0.join("base.txt"), "base\n").unwrap();
            directory.git(&["add", "."]);
            directory.git(&["commit", "-q", "-m", "base"]);
            directory.git(&["branch", "-m", "main"]);
            directory.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
            directory
        }

        fn git(&self, args: &[&str]) {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(&self.0)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    impl Drop for GitDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn branch_merge_detection_covers_ancestry_and_rebased_squash_merges() {
        let directory = GitDirectory::new();
        directory.git(&["branch", "ancestor"]);
        assert!(branch_is_merged(&directory.0, "ancestor").unwrap());

        directory.git(&["checkout", "-q", "-b", "unique"]);
        fs::write(directory.0.join("unique.txt"), "unique\n").unwrap();
        directory.git(&["add", "."]);
        directory.git(&["commit", "-q", "-m", "unique"]);
        assert!(!branch_is_merged(&directory.0, "unique").unwrap());

        directory.git(&["checkout", "-q", "main"]);
        directory.git(&["checkout", "-q", "-b", "rebased"]);
        fs::write(directory.0.join("rebased.txt"), "rebased\n").unwrap();
        directory.git(&["add", "."]);
        directory.git(&["commit", "-q", "-m", "rebased"]);
        directory.git(&["checkout", "-q", "main"]);
        directory.git(&["cherry-pick", "rebased"]);
        directory.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        assert!(
            branch_is_merged(&directory.0, "rebased").unwrap(),
            "an equivalent patch counted as unique"
        );
        assert!(!branch_is_merged(&directory.0, "missing").unwrap());
    }

    #[test]
    fn branch_is_merged_only_when_cherry_has_no_unique_patches() {
        assert!(cherry_has_no_unique_patches(""));
        assert!(cherry_has_no_unique_patches("- abc123\n- def456\n"));
        assert!(!cherry_has_no_unique_patches("+ abc123\n"));
        assert!(!cherry_has_no_unique_patches("- abc123\n+ def456\n"));
        assert!(!cherry_has_no_unique_patches("fatal: bad revision\n"));
    }

    #[test]
    fn worktree_resolution_refuses_the_main_checkout_and_falls_back_to_workspaces() {
        let worktrees = [
            orca::Worktree {
                path: PathBuf::from("/repo"),
                name: "VibeCut".into(),
                branch: Some("main".into()),
                is_main: true,
            },
            orca::Worktree {
                path: PathBuf::from("/workspaces/repo/ticket-58"),
                name: "ticket-58".into(),
                branch: Some("fix/ticket-58".into()),
                is_main: false,
            },
        ];
        assert!(
            resolve_worktree(&worktrees, None, "VibeCut")
                .unwrap_err()
                .contains("main checkout")
        );
        assert_eq!(
            resolve_worktree(&worktrees, None, "ticket-58").unwrap(),
            (
                PathBuf::from("/workspaces/repo/ticket-58"),
                Some("fix/ticket-58".into())
            )
        );
        assert_eq!(
            resolve_worktree(&worktrees, Some(Path::new("/workspaces/repo")), "dropped").unwrap(),
            (PathBuf::from("/workspaces/repo/dropped"), None)
        );
        assert!(resolve_worktree(&worktrees, None, "dropped").is_err());
    }
}
