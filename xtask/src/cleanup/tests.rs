//! Cleanup argument, dispatch-release, branch-safety and scope tests.
use super::dispatch::*;
use super::options::*;
use super::worktree::*;
use super::*;
use crate::orca::Worker;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

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
        worktree_remove_command(Some("Window"), Path::new("/workspaces/repo/ticket-58")).render(),
        "orca worktree rm --worktree path:/workspaces/repo/ticket-58 --delete-branch --environment Window"
    );
    assert_eq!(
        remote_branch_delete_command(Path::new("/repo"), "fix/p0-core-63-cleanup", "abc123")
            .render(),
        "git -C /repo push --force-with-lease=refs/heads/fix/p0-core-63-cleanup:abc123 origin --delete fix/p0-core-63-cleanup"
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
    fs::write(directory.0.join("main-only.txt"), "main-only\n").unwrap();
    directory.git(&["add", "."]);
    directory.git(&["commit", "-q", "-m", "advance main"]);
    directory.git(&["cherry-pick", "rebased"]);
    directory.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    assert!(
        !git_output(
            &directory.0,
            &["merge-base", "--is-ancestor", "rebased", "origin/main"]
        )
        .unwrap()
        .status
        .success()
    );
    assert!(
        branch_is_merged(&directory.0, "rebased").unwrap(),
        "an equivalent patch counted as unique"
    );
    assert!(!branch_is_merged(&directory.0, "missing").unwrap());

    let remote = GitDirectory::new();
    directory.git(&["remote", "add", "origin", remote.0.to_str().unwrap()]);
    for (branch, advanced, racing, allowed) in [
        ("merged-remote", false, false, true),
        ("advanced-remote", true, false, false),
        ("racing-remote", false, true, false),
    ] {
        directory.git(&["branch", branch, "main"]);
        directory.git(&["push", "-q", "origin", &format!("main:refs/heads/{branch}")]);
        if advanced {
            directory.git(&[
                "push",
                "-q",
                "--force",
                "origin",
                &format!("unique:refs/heads/{branch}"),
            ]);
        }
        assert!(branch_is_merged(&directory.0, branch).unwrap());
        let result =
            delete_remote_merged_branch(&directory.0, false, branch, &mut |command, dry| {
                if racing {
                    directory.git(&[
                        "push",
                        "-q",
                        "--force",
                        "origin",
                        &format!("unique:refs/heads/{branch}"),
                    ]);
                }
                perform(command, dry)
            });
        assert_eq!(result.is_ok(), allowed, "{branch}: {result:?}");
        let remaining = git_output(
            &remote.0,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ],
        )
        .unwrap();
        assert_eq!(remaining.status.success(), !allowed, "{branch}");
    }
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
