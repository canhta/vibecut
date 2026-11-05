//! Release settled Orca workers, remove ticket worktrees and prune build caches.

use crate::orca::{self, CommandLine, Worker};
use std::fs;
use std::path::{Path, PathBuf};

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

fn confirm_release(receipt: &serde_json::Value, dispatch: &str) -> Result<(), String> {
    let result = receipt
        .get("result")
        .ok_or("worker-release returned no result")?;
    if result.get("dispatchId").and_then(serde_json::Value::as_str) != Some(dispatch) {
        return Err("worker-release returned a different or missing dispatch ID".into());
    }
    match result.get("state").and_then(serde_json::Value::as_str) {
        Some("released" | "already_released") => Ok(()),
        state => Err(format!("worker-release did not confirm release: {state:?}")),
    }
}

fn perform(command: &CommandLine, dry_run: bool) -> Result<(), String> {
    if !dry_run
        && command
            .args
            .get(1)
            .is_some_and(|arg| arg == "worker-release")
    {
        let dispatch = command
            .args
            .get(3)
            .ok_or("worker-release has no dispatch ID")?;
        return confirm_release(&orca::command_json(command)?, dispatch);
    }
    orca::perform(command, dry_run)
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

type Perform<'a> = &'a mut dyn FnMut(&CommandLine, bool) -> Result<(), String>;

fn release_dispatch(
    environment: Option<&str>,
    dry_run: bool,
    workers: &[Worker],
    dispatch: &str,
    perform: Perform<'_>,
) -> Result<(), String> {
    let terminal = workers
        .iter()
        .find(|worker| worker.dispatch_id == dispatch)
        .and_then(|worker| worker.terminal.as_deref());
    if dry_run && terminal.is_none() {
        eprintln!("[cleanup] {dispatch}: no terminal reported; worker-release only");
    }
    // Closing the terminal is dependent on the release succeeding. A refused
    // release can mean the worker started again; never close its terminal in
    // that case. Independent target arguments still continue in `run_with`.
    perform(&release_command(environment, dispatch), dry_run)
        .map_err(|error| format!("worker-release failed; terminal left open: {error}"))?;
    if let Some(terminal) = terminal {
        perform(&terminal_close_command(environment, terminal), dry_run)?;
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
) -> Result<PathBuf, String> {
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
        return Ok(worktree.path.clone());
    }
    workspaces
        .map(|directory| directory.join(name))
        .ok_or_else(|| {
            format!("cannot resolve `{name}`: no Orca worktree or workspaces directory found")
        })
}

fn remove_worktree(
    environment: Option<&str>,
    dry_run: bool,
    worktrees: &[orca::Worktree],
    workspaces: Option<&Path>,
    name: &str,
    perform: Perform<'_>,
) -> Result<(), String> {
    let path = resolve_worktree(worktrees, workspaces, name)?;
    // A paired server owns the remote directory; `worktree rm` deletes its
    // ignored `.scratch` with the checkout. Locally the ignored tree goes first
    // so Git never has to force over untracked report files.
    if environment.is_none() {
        remove_scratch(&path, dry_run)?;
    }
    perform(&worktree_remove_command(environment, &path), dry_run)
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

#[cfg(unix)]
fn prune(dry_run: bool) -> Result<(), String> {
    crate::build::prune_cache(dry_run)
}

#[cfg(not(unix))]
fn prune(_dry_run: bool) -> Result<(), String> {
    eprintln!("[cleanup] shared build-cache prune is Unix-only; skipped");
    Ok(())
}

struct Discovery {
    worktrees: Result<Vec<orca::Worktree>, String>,
    workers: Result<Vec<Worker>, String>,
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
    F: FnMut(&CommandLine, bool) -> Result<(), String>,
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
    for target in &options.targets {
        let result = match target {
            Target::Dispatch(dispatch) => release_dispatch(
                environment,
                options.dry_run,
                &workers,
                dispatch,
                &mut perform,
            ),
            Target::Worktree(name) => remove_worktree(
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
    if let Err(error) = pull(main, options.dry_run) {
        eprintln!("[cleanup] pull: {error}");
        failures += 1;
    }
    if failures == 0 {
        Ok(())
    } else {
        Err(format!("{failures} cleanup step(s) failed"))
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
        orca::workers(environment)
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

    fn reported_worker() -> Vec<Worker> {
        vec![Worker {
            dispatch_id: "ctx_abc".into(),
            terminal: Some("term_x".into()),
        }]
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
            &mut |command, _| {
                commands.push(command.render());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            commands,
            [
                "orca orchestration worker-release --dispatch ctx_abc --json --full --environment Window",
                "orca terminal close --terminal term_x --environment Window",
            ]
        );
    }

    #[test]
    fn refused_release_leaves_the_terminal_open_and_reports_the_failure() {
        let workers = reported_worker();
        let mut commands = Vec::new();
        let error = release_dispatch(None, false, &workers, "ctx_abc", &mut |command, _| {
            commands.push(command.render());
            if command.args.iter().any(|arg| arg == "worker-release") {
                Err("refused: worker still running".into())
            } else {
                Ok(())
            }
        })
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
                &mut |command, _| {
                    commands.push(command.render());
                    if command
                        .args
                        .get(1)
                        .is_some_and(|arg| arg == "worker-release")
                    {
                        confirm_release(&receipt, "ctx_abc")
                    } else {
                        Ok(())
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
                Ok(())
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
    }

    #[test]
    fn worktree_resolution_refuses_the_main_checkout_and_falls_back_to_workspaces() {
        let worktrees = [
            orca::Worktree {
                path: PathBuf::from("/repo"),
                name: "VibeCut".into(),
                is_main: true,
            },
            orca::Worktree {
                path: PathBuf::from("/workspaces/repo/ticket-58"),
                name: "ticket-58".into(),
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
            PathBuf::from("/workspaces/repo/ticket-58")
        );
        assert_eq!(
            resolve_worktree(&worktrees, Some(Path::new("/workspaces/repo")), "dropped").unwrap(),
            PathBuf::from("/workspaces/repo/dropped")
        );
        assert!(resolve_worktree(&worktrees, None, "dropped").is_err());
    }
}
