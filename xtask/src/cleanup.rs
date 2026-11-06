//! Release settled Orca workers, remove ticket worktrees and prune build caches.
mod dispatch;
mod options;
mod worktree;
#[cfg(test)]
mod tests;

use dispatch::{ObservedWorker, discover_workers, perform, release_dispatch};
use options::{Options, Target, parse};
use crate::orca::{self, CommandLine};
use std::{collections::HashSet, path::Path};
use worktree::{pull, remove_worktree};

type Perform<'a> =
    &'a mut dyn FnMut(&CommandLine, bool) -> Result<Option<serde_json::Value>, String>;

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
