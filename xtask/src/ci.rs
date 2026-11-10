//! Workspace verification with a guarded hosted aggregate.
use crate::{cargo_check, ffmpeg, lint, process};
use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The one line a linked worktree gets instead of the hosted aggregate.
const REFUSED: &str = "cargo xtask ci is for hosted CI; in a worktree run: cargo xtask build -- cargo test -p <edited crates> and cargo xtask checks";

pub fn run(root: &Path, require_oracles: bool) -> Result<(), String> {
    guard(root, env::var_os("CI").as_deref())?;
    suite(root, require_oracles, true)
}

/// The one workspace test suite behind `cargo xtask test` and the aggregate
/// `cargo xtask ci`. Both callers keep their own oracle mode and fixture
/// receipt; the coverage, including the Windows feature builds, is the same.
pub(crate) fn suite(root: &Path, required: bool, with_lint: bool) -> Result<(), String> {
    let checks = ffmpeg::begin(required).and_then(|()| {
        if with_lint {
            lint(root)?;
        }
        workspace_tests(root, required)
    });
    let fixtures = ffmpeg::print_fixtures(root);
    checks.and(fixtures)
}

/// Refuse the hosted aggregate in a linked worktree unless `CI` is set and non-empty.
pub(crate) fn guard(root: &Path, ci: Option<&OsStr>) -> Result<(), String> {
    let hosted = ci.is_some_and(|value| !value.is_empty());
    if hosted || !linked(root)? {
        return Ok(());
    }
    Err(REFUSED.into())
}

/// Whether `root` is a linked worktree rather than the main checkout.
fn linked(root: &Path) -> Result<bool, String> {
    Ok(git_path(root, "--git-dir")? != git_path(root, "--git-common-dir")?)
}

fn git_path(root: &Path, key: &str) -> Result<PathBuf, String> {
    let bytes = process::checked(root, "git", &["rev-parse", "--path-format=absolute", key])?;
    let text = String::from_utf8(bytes).map_err(|_| format!("git {key} is not UTF-8"))?;
    Ok(PathBuf::from(text.trim_end_matches(['\r', '\n'])))
}

fn workspace_tests(root: &Path, required: bool) -> Result<(), String> {
    #[cfg(windows)]
    for feature in ["whisper", "gipformer"] {
        cargo_check(
            root,
            &[
                "test",
                "-p",
                "ai",
                "--lib",
                "--no-default-features",
                "--features",
                feature,
            ],
            required,
        )?;
    }
    cargo_check(root, &["test", "--workspace"], required)
}

#[cfg(test)]
mod tests {
    use super::{REFUSED, guard};
    use crate::process;
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};

    fn git(root: &Path, args: &[&str]) -> Vec<u8> {
        let mut all = vec![
            "-c",
            "user.name=xtask",
            "-c",
            "user.email=xtask@example.com",
            "-c",
            "commit.gpgsign=false",
        ];
        all.extend_from_slice(args);
        process::checked(root, "git", &all).unwrap()
    }

    fn repository() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "--quiet"]);
        directory
    }

    fn linked_worktree(main: &Path) -> PathBuf {
        git(
            main,
            &[
                "commit",
                "--allow-empty",
                "--no-gpg-sign",
                "--quiet",
                "-m",
                "init",
            ],
        );
        let linked = main.join("linked");
        git(
            main,
            &["worktree", "add", "--quiet", linked.to_str().unwrap()],
        );
        linked
    }

    type Case<'a> = (&'a str, &'a Path, Option<&'a OsStr>, Result<(), &'a str>);

    #[test]
    fn guard_refuses_hosted_ci_in_a_linked_worktree_without_ci() {
        let main = repository();
        let linked = linked_worktree(main.path());
        let cases: [Case; 4] = [
            ("linked worktree, CI unset", &linked, None, Err(REFUSED)),
            (
                "linked worktree, CI empty",
                &linked,
                Some(OsStr::new("")),
                Err(REFUSED),
            ),
            (
                "linked worktree, CI=1",
                &linked,
                Some(OsStr::new("1")),
                Ok(()),
            ),
            ("main checkout, CI unset", main.path(), None, Ok(())),
        ];
        for (name, root, ci, expected) in cases {
            assert_eq!(
                guard(root, ci),
                expected.map_err(str::to_string),
                "case: {name}"
            );
        }
    }
}
