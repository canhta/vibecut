//! File-level structure rules: responsibility line, banned names and folder width.
use super::{
    BANNED, DOC, FOLDER, FOLDER_LIMIT, INTERFACE, NAME, Violation, has_test_dir, is_test_source,
    syntax::{is_cfg_test, item_attrs},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
use syn::{Item, ItemMod, visit::Visit};

pub(super) fn leading_doc(root: &Path, path: &str) -> Result<Option<Violation>, String> {
    let text = fs::read_to_string(root.join(path)).map_err(|e| format!("{path}: {e}"))?;
    let first = text
        .trim_start_matches('\u{feff}')
        .lines()
        .next()
        .unwrap_or_default();
    let Some(line) = first.trim_start().strip_prefix("//!") else {
        return Ok(Some(Violation {
            path: path.to_owned(),
            rule: DOC,
            detail: "first line is not a //! responsibility line".into(),
        }));
    };
    if line.trim().is_empty() {
        return Ok(Some(Violation {
            path: path.to_owned(),
            rule: DOC,
            detail: "//! responsibility line is empty".into(),
        }));
    }
    Ok(None)
}

pub(super) fn mod_file(path: &str) -> Option<Violation> {
    if Path::new(path)
        .file_name()
        .is_some_and(|name| name == "mod.rs")
    {
        return Some(Violation {
            path: path.to_owned(),
            rule: INTERFACE,
            detail: "module interface is `mod.rs`; use `foo.rs` beside `foo/`".into(),
        });
    }
    None
}

pub(super) fn file_name(path: &str) -> Option<Violation> {
    let name = Path::new(path).file_stem()?.to_str()?;
    if name == "mod" || !banned(name) {
        return None;
    }
    Some(Violation {
        path: path.to_owned(),
        rule: NAME,
        detail: format!("module name `{name}` is banned"),
    })
}

pub(super) fn inline_names(root: &Path, path: &str) -> Result<Vec<Violation>, String> {
    let text = fs::read_to_string(root.join(path)).map_err(|e| format!("{path}: {e}"))?;
    let parsed = syn::parse_file(&text).map_err(|e| format!("{path}: invalid Rust: {e}"))?;
    let mut mods = ModNames(Vec::new());
    mods.visit_file(&parsed);
    Ok(mods
        .0
        .into_iter()
        .map(|name| Violation {
            path: path.to_owned(),
            rule: NAME,
            detail: format!("module name `{name}` is banned"),
        })
        .collect())
}

pub(super) fn folder_rules(tracked: &[String]) -> BTreeSet<Violation> {
    let mut dirs = BTreeSet::new();
    let mut children: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for path in tracked {
        if is_test_source(path) {
            continue;
        }
        let parts: Vec<&str> = path.split('/').collect();
        for index in 0..parts.len().saturating_sub(1) {
            let Some(prefix) = parts.get(..=index) else {
                continue;
            };
            let dir = prefix.join("/");
            let Some(child) = parts.get(index + 1) else {
                continue;
            };
            dirs.insert(dir.clone());
            children.entry(dir).or_default().insert((*child).to_owned());
        }
    }
    let mut violations = BTreeSet::new();
    for dir in dirs.iter().filter(|dir| is_source_dir(dir)) {
        if let Some(name) = dir.rsplit('/').next()
            && banned(name)
        {
            violations.insert(Violation {
                path: dir.clone(),
                rule: NAME,
                detail: format!("folder name `{name}` is banned"),
            });
        }
        let Some(entries) = children.get(dir) else {
            continue;
        };
        let count = entries
            .iter()
            .filter(|entry| entry.as_str() != "mod.rs")
            .filter(|entry| {
                entry
                    .strip_suffix(".rs")
                    .is_none_or(|stem| !entries.contains(stem))
            })
            .count();
        if count > FOLDER_LIMIT {
            violations.insert(Violation {
                path: dir.clone(),
                rule: FOLDER,
                detail: format!("{count} direct children (limit {FOLDER_LIMIT})"),
            });
        }
    }
    violations
}

/// Banned names also apply to module trees under `tests/`, `examples/` and `benches/`.
pub(super) fn test_folder_rules(tracked: &[String]) -> BTreeSet<Violation> {
    let mut dirs = BTreeSet::new();
    for path in tracked {
        if !has_test_dir(path) {
            continue;
        }
        let parts: Vec<&str> = path.split('/').collect();
        for end in 1..parts.len() {
            let dir = parts.get(..end).unwrap_or_default().join("/");
            if has_test_dir(&dir) {
                dirs.insert(dir);
            }
        }
    }
    let mut violations = BTreeSet::new();
    for dir in dirs {
        let Some(name) = dir.rsplit('/').next().map(str::to_owned) else {
            continue;
        };
        if banned(&name) {
            violations.insert(Violation {
                path: dir,
                rule: NAME,
                detail: format!("module name `{name}` is banned"),
            });
        }
    }
    violations
}

pub(super) fn banned(name: &str) -> bool {
    BANNED.contains(&name.to_ascii_lowercase().as_str())
}

pub(super) fn is_source_dir(dir: &str) -> bool {
    dir == "xtask/src" || dir.ends_with("/src") || dir.contains("/src/")
}

/// Inline `mod` blocks (and `#[path]` modules) whose name is banned; file modules are caught by name.
pub(super) struct ModNames(Vec<String>);

impl<'ast> Visit<'ast> for ModNames {
    fn visit_item(&mut self, item: &'ast Item) {
        if !is_cfg_test(item_attrs(item)) {
            syn::visit::visit_item(self, item);
        }
    }

    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        let name = item.ident.to_string();
        let path_attr = item.attrs.iter().any(|attr| attr.path().is_ident("path"));
        if (item.content.is_some() || path_attr) && banned(&name) {
            self.0.push(name);
        }
        syn::visit::visit_item_mod(self, item);
    }
}
