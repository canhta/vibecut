//! Each crate and app README maps every folder module below `src/`, and each
//! folder module carries its own contract README.
use super::{MAP, Violation, test_path};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

#[derive(Default)]
struct Modules {
    folders: BTreeSet<String>,
    all: BTreeSet<String>,
}

pub(super) fn check(root: &Path) -> Result<BTreeSet<Violation>, String> {
    let tracked = super::tracked(root, &["crates", "apps"])?;
    let readmes: Vec<&String> = tracked
        .iter()
        .filter(|path| package_readme(path).is_some())
        .collect();
    let mut packages: BTreeMap<String, Modules> = BTreeMap::new();
    for readme in &readmes {
        let Some(package) = package_readme(readme) else {
            continue;
        };
        packages.entry(package).or_default();
    }
    let names: Vec<String> = packages.keys().cloned().collect();
    for package in names {
        let prefix = format!("{package}/src/");
        for file in &tracked {
            let Some(rest) = file.strip_prefix(&prefix) else {
                continue;
            };
            let Some(module) = source_module(rest) else {
                continue;
            };
            let Some(modules) = packages.get_mut(&package) else {
                continue;
            };
            modules.all.insert(module);
            let mut parts: Vec<&str> = rest.split('/').collect();
            parts.pop();
            for end in 1..=parts.len() {
                let folder = parts.get(..end).unwrap_or_default().join("/");
                modules.folders.insert(folder.clone());
                modules.all.insert(folder);
            }
        }
    }
    let mut violations = BTreeSet::new();
    let tracked: BTreeSet<&str> = tracked.iter().map(String::as_str).collect();
    for readme in readmes {
        let Some(package) = package_readme(readme) else {
            continue;
        };
        let text = fs::read_to_string(root.join(readme)).map_err(|e| format!("{readme}: {e}"))?;
        let (has_map, rows) = parse_map(&text);
        let empty = Modules::default();
        let modules = packages.get(&package).unwrap_or(&empty);
        if !has_map {
            violations.insert(Violation {
                path: readme.clone(),
                rule: MAP,
                detail: "README has no module map table".into(),
            });
            continue;
        }
        for folder in &modules.folders {
            if !rows.contains(folder) {
                violations.insert(Violation {
                    path: readme.clone(),
                    rule: MAP,
                    detail: format!("module map has no row for folder `{folder}`"),
                });
            }
            if !tracked.contains(format!("{package}/src/{folder}/README.md").as_str()) {
                violations.insert(Violation {
                    path: readme.clone(),
                    rule: MAP,
                    detail: format!("folder module `{folder}` has no README"),
                });
            }
        }
        for row in &rows {
            if !modules.all.contains(row) {
                violations.insert(Violation {
                    path: readme.clone(),
                    rule: MAP,
                    detail: format!("module map row `{row}` names no module under src"),
                });
            }
        }
    }
    Ok(violations)
}

fn package_readme(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    let [root, name, "README.md"] = parts.as_slice() else {
        return None;
    };
    if !matches!(*root, "crates" | "apps") {
        return None;
    }
    Some(format!("{root}/{name}"))
}

/// A crate-root module path from a file below `src/`; `bin/` holds binaries, not modules.
fn source_module(rest: &str) -> Option<String> {
    if test_path(rest) || rest.starts_with("bin/") {
        return None;
    }
    let stem = rest.strip_suffix(".rs")?;
    let mut parts: Vec<&str> = stem.split('/').collect();
    let file = parts.pop()?;
    if matches!(file, "lib" | "main") {
        return None;
    }
    if file != "mod" {
        parts.push(file);
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

/// Returns whether a module map table exists and every row name it lists.
fn parse_map(text: &str) -> (bool, BTreeSet<String>) {
    let mut has_map = false;
    let mut rows = BTreeSet::new();
    let mut table: Option<bool> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') {
            table = None;
            continue;
        }
        match table {
            Some(true) if !separator(trimmed) => {
                if let Some(name) = row_name(trimmed) {
                    rows.insert(name);
                }
            }
            Some(_) => {}
            None => {
                let is_map = map_header(trimmed);
                has_map |= is_map;
                table = Some(is_map);
            }
        }
    }
    (has_map, rows)
}

fn map_header(line: &str) -> bool {
    line.trim_matches('|')
        .split('|')
        .map(str::trim)
        .map(|cell| cell.trim_matches('`').to_ascii_lowercase())
        .any(|cell| matches!(cell.as_str(), "folder" | "module" | "modules" | "owner"))
}

fn separator(line: &str) -> bool {
    line.trim_matches('|').split('|').all(|cell| {
        let cell = cell.trim();
        !cell.is_empty() && cell.chars().all(|c| matches!(c, '-' | ':' | ' ' | '\t'))
    })
}

fn row_name(line: &str) -> Option<String> {
    let first = line.trim_matches('|').split('|').next()?.trim();
    let token = match first.find('`') {
        Some(start) => first.get(start + 1..)?.split('`').next()?,
        None => first.split_whitespace().next()?,
    };
    normalize(token)
}

fn normalize(token: &str) -> Option<String> {
    let token = token.trim().trim_end_matches('/').trim_end_matches(".rs");
    let token = token.strip_prefix("src/").unwrap_or(token);
    if token.is_empty()
        || !token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/'))
    {
        return None;
    }
    Some(token.to_owned())
}
