//! Documentation gate: the docs index, README module maps, tracker-free prose and
//! links that resolve.
//!
//! Tracks the mechanical parts of the documentation rules: every tracked page is
//! linked from `docs/README.md`, every crate and app README maps its folder
//! modules, tracked Markdown outside `.scratch/` carries no tracker ids or
//! reference-checkout source paths, and every relative link and anchor resolves.
//! Forbidden private names belong to `xtask::policy::names` and run before this
//! gate. Current violations live in `xtask/docs-baseline.txt`, which only shrinks.
use baseline::baseline;
use std::{collections::BTreeSet, path::Path};

const ANCHOR: &str = "anchor";
const BASELINE: &str = "xtask/docs-baseline.txt";
const INDEX: &str = "index";
const LINK: &str = "link";
const MAP: &str = "map";
const SOURCE: &str = "source";
const TEXT: &str = "text";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Violation {
    path: String,
    rule: &'static str,
    detail: String,
}

impl Violation {
    fn message(&self) -> String {
        format!("{}: [{}] docs check: {}", self.path, self.rule, self.detail)
    }

    fn stale_message(&self) -> String {
        format!(
            "{}: [{}] docs baseline entry no longer occurs; remove it from {BASELINE} ({})",
            self.path, self.rule, self.detail
        )
    }
}

pub fn check(root: &Path) -> Result<(), String> {
    let tracked = markdown(root)?;
    let mut violations = BTreeSet::new();
    violations.extend(index::check(root, &tracked)?);
    violations.extend(map::check(root)?);
    violations.extend(text::check(root, &tracked)?);
    violations.extend(links::check(root, &tracked)?);
    let baseline = baseline(root)?;
    let mut errors: Vec<String> = violations
        .difference(&baseline)
        .map(Violation::message)
        .collect();
    errors.extend(
        baseline
            .difference(&violations)
            .map(Violation::stale_message),
    );
    errors.sort();
    if errors.is_empty() {
        eprintln!(
            "docs: ok ({} tracked Markdown files, {} baselined)",
            tracked.len(),
            baseline.len()
        );
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

fn tracked(root: &Path, paths: &[&str]) -> Result<Vec<String>, String> {
    let mut args = vec!["ls-files", "-z", "--"];
    args.extend(paths);
    let out = crate::process::checked(root, "git", &args)?;
    let mut files = Vec::new();
    for raw in out.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        let path =
            std::str::from_utf8(raw).map_err(|_| "git ls-files returned a non-UTF-8 path")?;
        if path.starts_with(".scratch/") {
            continue;
        }
        files.push(path.to_owned());
    }
    files.sort();
    Ok(files)
}

/// Every tracked Markdown page. Filtering in Rust keeps the gate independent of
/// the git build's pathspec glob rules, which differ between hosted runners.
fn markdown(root: &Path) -> Result<Vec<String>, String> {
    let mut files = tracked(root, &[])?;
    files.retain(|path| path.ends_with(".md"));
    Ok(files)
}

/// Test support trees mirror source modules but follow no documentation rule.
fn test_path(path: &str) -> bool {
    path.split('/')
        .any(|part| matches!(part, "tests" | "examples" | "benches"))
}

mod baseline;
mod index;
mod links;
mod map;
#[cfg(test)]
mod tests;
mod text;
