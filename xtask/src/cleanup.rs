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

fn release_commands(
    environment: Option<&str>,
    dispatch: &str,
    terminal: Option<&str>,
) -> Vec<CommandLine> {
    let mut commands = vec![orca::command(
        environment,
        &["orchestration", "worker-release", "--dispatch", dispatch],
    )];
    if let Some(terminal) = terminal {
        commands.push(orca::command(
            environment,
            &["terminal", "close", "--terminal", terminal],
        ));
    }
    commands
}

fn worktree_remove_command(environment: Option<&str>, path: &Path) -> CommandLine {
    let selector = format!("path:{}", path.display());
    orca::command(
        environment,
        &["worktree", "rm", "--worktree", &selector, "--delete-branch"],
    )
}

fn release_dispatch(
    environment: Option<&str>,
    dry_run: bool,
    workers: &[Worker],
    dispatch: &str,
) -> Result<(), String> {
    let terminal = workers
        .iter()
        .find(|worker| worker.dispatch_id == dispatch)
        .and_then(|worker| worker.terminal.as_deref());
    if dry_run && terminal.is_none() {
        eprintln!("[cleanup] {dispatch}: no terminal reported; worker-release only");
    }
    let mut errors = Vec::new();
    for command in release_commands(environment, dispatch, terminal) {
        if let Err(error) = orca::perform(&command, dry_run) {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
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
) -> Result<(), String> {
    let path = resolve_worktree(worktrees, workspaces, name)?;
    // A paired server owns the remote directory; `worktree rm` deletes its
    // ignored `.scratch` with the checkout. Locally the ignored tree goes first
    // so Git never has to force over untracked report files.
    if environment.is_none() {
        remove_scratch(&path, dry_run)?;
    }
    orca::perform(&worktree_remove_command(environment, &path), dry_run)
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

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let options = parse(args)?;
    let environment = options.environment.as_deref();
    let main = crate::checkout::main(root)?;
    let worktrees = match orca::worktrees(environment) {
        Ok(worktrees) => worktrees,
        Err(error) => {
            eprintln!("[cleanup] worktree list: {error}");
            Vec::new()
        }
    };
    let workspaces = orca::workspaces_dir(&worktrees);
    let needs_workers = options
        .targets
        .iter()
        .any(|target| matches!(target, Target::Dispatch(_)));
    let workers = if needs_workers {
        match orca::workers(environment) {
            Ok(workers) => workers,
            Err(error) => {
                eprintln!("[cleanup] worker list: {error}");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    let mut failures = 0usize;
    for target in &options.targets {
        let result = match target {
            Target::Dispatch(dispatch) => {
                release_dispatch(environment, options.dry_run, &workers, dispatch)
            }
            Target::Worktree(name) => remove_worktree(
                environment,
                options.dry_run,
                &worktrees,
                workspaces.as_deref(),
                name,
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
    if let Err(error) = pull(&main, options.dry_run) {
        eprintln!("[cleanup] pull: {error}");
        failures += 1;
    }
    if failures == 0 {
        Ok(())
    } else {
        Err(format!("{failures} cleanup step(s) failed"))
    }
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

    #[test]
    fn dispatch_commands_release_then_close_the_reported_terminal() {
        let commands = release_commands(Some("Window"), "ctx_abc", Some("term_x"));
        assert_eq!(
            commands.iter().map(CommandLine::render).collect::<Vec<_>>(),
            [
                "orca orchestration worker-release --dispatch ctx_abc --environment Window",
                "orca terminal close --terminal term_x --environment Window",
            ]
        );
        assert_eq!(
            release_commands(None, "ctx_abc", None)
                .iter()
                .map(CommandLine::render)
                .collect::<Vec<_>>(),
            ["orca orchestration worker-release --dispatch ctx_abc"]
        );
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
