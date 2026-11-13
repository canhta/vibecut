//! Module structure and dependency-direction gate for `docs/engineering/structure.md`.
//!
//! Tracks the mechanical parts of the structure rules: every source file states
//! its responsibility in a leading `//!` line, module and folder names use the
//! concept's name, folders stay narrow, intra-crate paths follow the layer
//! table in `xtask/modules.toml`, and the shared editor context and the areas
//! never name the frame's `Editor` record. Current violations live in
//! `xtask/modules-baseline.txt`, which only shrinks; the judgement calls in
//! rule 7 stay with the reviewer.
use std::{collections::BTreeSet, path::Path};

use baseline::baseline;
use config::config;
use frame_type::frame_types;
use imports::{control_test_order, imports};
use layout::{file_name, folder_rules, inline_names, leading_doc, mod_file, test_folder_rules};

const CONFIG: &str = "xtask/modules.toml";
const BASELINE: &str = "xtask/modules-baseline.txt";
const FOLDER_LIMIT: usize = 12;
const BANNED: &[&str] = &[
    "utils", "common", "manager", "helpers", "misc", "core", "types",
];
const TEST_DIRS: &[&str] = &["tests", "examples", "benches"];
const DOC: &str = "doc";
const NAME: &str = "name";
const FOLDER: &str = "folder";
const INTERFACE: &str = "interface";
const LAYER: &str = "layer";
const FRAME_TYPE: &str = "frame-type";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Violation {
    path: String,
    rule: &'static str,
    detail: String,
}

impl Violation {
    fn message(&self) -> String {
        format!(
            "{}: [{}] structure rule {}: {}",
            self.path,
            self.rule,
            rule_number(self.rule),
            self.detail
        )
    }

    fn stale_message(&self) -> String {
        format!(
            "{}: [{}] structure rule {}: baseline entry no longer occurs; remove it from {BASELINE} ({})",
            self.path,
            self.rule,
            rule_number(self.rule),
            self.detail
        )
    }
}

pub fn check(root: &Path) -> Result<(), String> {
    let config = config(root)?;
    let tracked = tracked(root)?;
    let sources: Vec<&str> = tracked
        .iter()
        .map(String::as_str)
        .filter(|path| path.ends_with(".rs"))
        .collect();
    config.validate(&sources)?;
    let mut violations = BTreeSet::new();
    for path in &sources {
        violations.extend(mod_file(path));
        violations.extend(file_name(path));
        violations.extend(inline_names(root, path)?);
        if !is_test_source(path) {
            violations.extend(leading_doc(root, path)?);
        }
    }
    violations.extend(folder_rules(&tracked));
    violations.extend(test_folder_rules(&tracked));
    violations.extend(imports(root, &config, &sources)?);
    violations.extend(frame_types(root, &config, &sources)?);
    control_test_order(root)?;
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
            "modules: ok ({} tracked files, {} Rust files, {} baselined)",
            tracked.len(),
            sources.len(),
            baseline.len()
        );
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

fn has_test_dir(path: &str) -> bool {
    path.split('/').any(|part| TEST_DIRS.contains(&part))
}

/// Source-tree test modules live beside production code but follow no structure rule.
fn is_test_source(path: &str) -> bool {
    if has_test_dir(path) {
        return true;
    }
    Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem == "tests" || stem.ends_with("_tests"))
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn tracked(root: &Path) -> Result<Vec<String>, String> {
    let out = crate::process::checked(
        root,
        "git",
        &["ls-files", "-z", "--", "apps", "crates", "xtask"],
    )?;
    let mut files = Vec::new();
    for raw in out.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        let path =
            std::str::from_utf8(raw).map_err(|_| "git ls-files returned a non-UTF-8 path")?;
        files.push(path.to_owned());
    }
    files.sort();
    Ok(files)
}

fn rule_number(rule: &str) -> u8 {
    match rule {
        DOC | NAME => 1,
        FOLDER => 2,
        INTERFACE => 3,
        LAYER => 4,
        FRAME_TYPE => 5,
        _ => 0,
    }
}

mod baseline;
mod config;
mod frame_type;
mod imports;
mod layout;
mod syntax;
#[cfg(test)]
mod tests;
