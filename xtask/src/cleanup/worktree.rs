//! Remove ticket worktrees and delete only branches proven merged into origin/main.
use super::Perform;
use crate::orca::{self, CommandLine};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Output,
};

pub(super) fn worktree_remove_command(environment: Option<&str>, path: &Path) -> CommandLine {
    let selector = format!("path:{}", path.display());
    orca::command(
        environment,
        &["worktree", "rm", "--worktree", &selector, "--delete-branch"],
    )
}
pub(super) fn remove_scratch(path: &Path, dry_run: bool) -> Result<(), String> {
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

pub(super) fn resolve_worktree(
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

pub(super) fn git_command(main: &Path, args: &[&str]) -> CommandLine {
    let mut arguments = vec!["-C".to_owned(), main.to_string_lossy().into_owned()];
    arguments.extend(args.iter().map(|arg| (*arg).to_owned()));
    CommandLine::new("git", arguments)
}

pub(super) fn git_output(main: &Path, args: &[&str]) -> Result<Output, String> {
    git_command(main, args).run()
}

/// Branch names come from Orca data, but they still reach Git as option-like
/// arguments. Reject anything that is not a plain local branch ref before use.
pub(super) fn valid_branch_name(branch: &str) -> bool {
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
pub(super) fn cherry_has_no_unique_patches(output: &str) -> bool {
    output.lines().all(|line| {
        let line = line.trim_end();
        line.is_empty() || line.starts_with('-')
    })
}

pub(super) fn branch_is_merged(main: &Path, branch: &str) -> Result<bool, String> {
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
    reference_is_merged(main, &reference)
}

pub(super) fn reference_is_merged(main: &Path, reference: &str) -> Result<bool, String> {
    if git_output(
        main,
        &["merge-base", "--is-ancestor", reference, "origin/main"],
    )?
    .status
    .success()
    {
        return Ok(true);
    }
    let cherry = git_output(main, &["cherry", "origin/main", reference])?;
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

pub(super) fn remote_branch_delete_command(main: &Path, branch: &str, tip: &str) -> CommandLine {
    let lease = format!("--force-with-lease=refs/heads/{branch}:{tip}");
    git_command(main, &["push", &lease, "origin", "--delete", branch])
}

pub(super) fn local_branch_delete_command(main: &Path, branch: &str) -> CommandLine {
    git_command(main, &["branch", "-D", branch])
}

pub(super) fn delete_remote_merged_branch(
    main: &Path,
    dry_run: bool,
    branch: &str,
    perform: Perform<'_>,
) -> Result<(), String> {
    // Deleting the remote name is a no-op if somebody already removed it.
    let remote = git_output(
        main,
        &[
            "ls-remote",
            "--exit-code",
            "--heads",
            "origin",
            &format!("refs/heads/{branch}"),
        ],
    )?;
    if remote.status.success() {
        // A same-named local branch can be older than the actual remote tip.
        // Fetch the advertised object, validate that exact tip, and lease the
        // deletion so concurrent pushes cannot lose unmerged work.
        let listing = String::from_utf8_lossy(&remote.stdout);
        let tip = listing
            .split_whitespace()
            .next()
            .ok_or("missing remote branch tip")?;
        let fetched = git_output(main, &["fetch", "--no-tags", "origin", tip])?;
        if !fetched.status.success() {
            return Err(format!("cannot fetch remote branch tip for {branch}"));
        }
        if !reference_is_merged(main, tip)? {
            return Err(format!(
                "remote branch {branch} has unique patches; retained"
            ));
        }
        perform(&remote_branch_delete_command(main, branch, tip), dry_run)?;
    } else if remote.status.code() != Some(2) {
        return Err(format!(
            "git ls-remote failed: {}",
            String::from_utf8_lossy(&remote.stderr).trim()
        ));
    }
    Ok(())
}

pub(super) fn delete_local_merged_branch(
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

pub(super) fn remove_worktree(
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

pub(super) fn pull(main: &Path, dry_run: bool) -> Result<(), String> {
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
