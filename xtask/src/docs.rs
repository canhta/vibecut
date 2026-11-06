//! Documentation gate: the docs index, README module maps and tracker-free prose.
//!
//! Tracks the mechanical parts of the documentation rules: every tracked page is
//! linked from `docs/README.md`, every crate and app README maps its folder
//! modules, and tracked Markdown outside `.scratch/` carries no tracker ids or
//! private host and agent names. Current violations live in
//! `xtask/docs-baseline.txt`, which only shrinks.
use baseline::baseline;
use std::{collections::BTreeSet, path::Path};

const BASELINE: &str = "xtask/docs-baseline.txt";
const INDEX: &str = "index";
const MAP: &str = "map";
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
    let tracked = tracked(root, &["*.md"])?;
    let mut violations = BTreeSet::new();
    violations.extend(index::check(root, &tracked)?);
    violations.extend(map::check(root)?);
    violations.extend(text::check(root, &tracked)?);
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
    let out = crate::checked(root, "git", &args)?;
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

/// Test support trees mirror source modules but follow no documentation rule.
fn test_path(path: &str) -> bool {
    path.split('/')
        .any(|part| matches!(part, "tests" | "examples" | "benches"))
}

mod baseline;
mod index;
mod map;
#[cfg(test)]
mod tests;
mod text;
