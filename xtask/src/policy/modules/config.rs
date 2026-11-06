//! The `xtask/modules.toml` layer tables, crate root and folder, and the module paths they apply to.
use super::{CONFIG, is_test_source};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

#[derive(Clone)]
pub(super) struct Placement {
    pub(super) layer: usize,
    pub(super) layer_name: String,
    pub(super) group: String,
}

pub(super) struct FolderTable {
    pub(super) path: String,
    pub(super) prefix: Vec<String>,
    pub(super) placements: BTreeMap<String, Placement>,
}

pub(super) struct CrateTable {
    pub(super) path: String,
    pub(super) placements: BTreeMap<String, Placement>,
    pub(super) folders: Vec<FolderTable>,
}

pub(super) struct Table {
    pub(super) crates: Vec<CrateTable>,
}

impl Table {
    pub(super) fn crate_for(&self, file: &str) -> Option<&CrateTable> {
        self.crates
            .iter()
            .filter(|entry| {
                file.strip_prefix(&entry.path)
                    .is_some_and(|rest| rest.starts_with('/'))
            })
            .max_by_key(|entry| entry.path.len())
    }

    pub(super) fn validate(&self, sources: &[&str]) -> Result<(), String> {
        for entry in &self.crates {
            validate_root(entry, sources)?;
            for folder in &entry.folders {
                validate_folder(entry, folder, sources)?;
            }
        }
        Ok(())
    }
}

fn validate_root(entry: &CrateTable, sources: &[&str]) -> Result<(), String> {
    let mut present = BTreeSet::new();
    for file in sources {
        if is_test_source(file) {
            continue;
        }
        let Some(module) = module_path(&entry.path, file) else {
            continue;
        };
        let Some(top) = module.first() else {
            continue;
        };
        if !entry.placements.contains_key(top) {
            return Err(format!(
                "{CONFIG}: `{}` top-level module `{top}` has no layer and group",
                entry.path
            ));
        }
        present.insert(top.clone());
    }
    if let Some(gone) = entry
        .placements
        .keys()
        .find(|name| !present.contains(*name))
    {
        return Err(format!(
            "{CONFIG}: `{}` places module `{gone}`, which no longer exists; remove it",
            entry.path
        ));
    }
    Ok(())
}

fn validate_folder(
    entry: &CrateTable,
    folder: &FolderTable,
    sources: &[&str],
) -> Result<(), String> {
    let mut present = BTreeSet::new();
    for file in sources {
        if is_test_source(file) {
            continue;
        }
        let Some(module) = module_path(&entry.path, file) else {
            continue;
        };
        if !inside(folder, &module) {
            continue;
        }
        let Some(child) = module.get(folder.prefix.len()) else {
            continue;
        };
        if !folder.placements.contains_key(child) {
            return Err(format!(
                "{CONFIG}: `{}` folder `{}` module `{child}` has no layer and group",
                entry.path, folder.path
            ));
        }
        present.insert(child.clone());
    }
    if let Some(gone) = folder
        .placements
        .keys()
        .find(|name| !present.contains(*name))
    {
        return Err(format!(
            "{CONFIG}: `{}` folder `{}` places module `{gone}`, which no longer exists; remove it",
            entry.path, folder.path
        ));
    }
    Ok(())
}

pub(super) fn config(root: &Path) -> Result<Table, String> {
    let path = root.join(CONFIG);
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value: toml::Value = toml::from_str(&text).map_err(|e| format!("{CONFIG}: {e}"))?;
    let mut crates = Vec::new();
    let mut paths = BTreeSet::new();
    for entry in array(&value, "crate")? {
        let name = text_value(entry, "name")?;
        let context = format!("crate `{name}`");
        let path = text_value(entry, "path")?.trim_end_matches('/').to_owned();
        if path.is_empty() || !paths.insert(path.clone()) {
            return Err(format!("{CONFIG}: duplicate or empty crate path `{path}`"));
        }
        let root_placements = placements(entry, &context)?;
        let mut folders = Vec::new();
        let mut folder_paths = BTreeSet::new();
        for folder in array(entry, "folder")? {
            let folder_path = text_value(folder, "path")?.trim_end_matches('/').to_owned();
            let prefix = folder_prefix(&folder_path, &context)?;
            if !folder_paths.insert(folder_path.clone()) {
                return Err(format!(
                    "{CONFIG}: {context} repeats folder path `{folder_path}`"
                ));
            }
            let folder_context = format!("{context} folder `{folder_path}`");
            folders.push(FolderTable {
                path: folder_path,
                prefix,
                placements: placements(folder, &folder_context)?,
            });
        }
        crates.push(CrateTable {
            path,
            placements: root_placements,
            folders,
        });
    }
    Ok(Table { crates })
}

fn placements(entry: &toml::Value, context: &str) -> Result<BTreeMap<String, Placement>, String> {
    let mut layers = Vec::new();
    let mut groups = BTreeSet::new();
    for layer in array(entry, "layer")? {
        let layer_name = text_value(layer, "name")?;
        let mut layer_groups = Vec::new();
        for group in array(layer, "groups")? {
            let group = group
                .as_str()
                .ok_or_else(|| {
                    format!("{CONFIG}: {context} layer `{layer_name}` has a non-string group")
                })?
                .to_owned();
            if !groups.insert(group.clone()) {
                return Err(format!(
                    "{CONFIG}: {context} repeats group `{group}` across layers"
                ));
            }
            layer_groups.push(group);
        }
        layers.push((layer_name, layer_groups));
    }
    let mut placements = BTreeMap::new();
    for module in array(entry, "module")? {
        let module_name = text_value(module, "name")?;
        if module_name.is_empty() || module_name.contains('/') {
            return Err(format!(
                "{CONFIG}: {context} module `{module_name}` must be one path segment"
            ));
        }
        let group = text_value(module, "group")?;
        let layer = layers
            .iter()
            .position(|(_, groups)| groups.iter().any(|name| name == &group))
            .ok_or_else(|| {
                format!(
                    "{CONFIG}: {context} module `{module_name}` group `{group}` is not in any layer"
                )
            })?;
        let layer_name = layers
            .get(layer)
            .map(|(name, _)| name.clone())
            .unwrap_or_default();
        if placements
            .insert(
                module_name.clone(),
                Placement {
                    layer,
                    layer_name,
                    group,
                },
            )
            .is_some()
        {
            return Err(format!(
                "{CONFIG}: {context} repeats module `{module_name}`"
            ));
        }
    }
    Ok(placements)
}

/// Folder paths are crate-relative Rust source directories, e.g. `src/editor`.
fn folder_prefix(path: &str, context: &str) -> Result<Vec<String>, String> {
    let rest = path.strip_prefix("src/").ok_or_else(|| {
        format!("{CONFIG}: {context} folder path `{path}` must start with `src/`")
    })?;
    let parts: Vec<&str> = rest.split('/').collect();
    if parts.is_empty()
        || parts
            .iter()
            .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return Err(format!(
            "{CONFIG}: {context} folder path `{path}` must name a directory below `src/`"
        ));
    }
    Ok(parts.into_iter().map(str::to_owned).collect())
}

pub(super) fn inside(folder: &FolderTable, module: &[String]) -> bool {
    module.len() > folder.prefix.len() && module.starts_with(&folder.prefix)
}

pub(super) fn array<'a>(table: &'a toml::Value, key: &str) -> Result<&'a [toml::Value], String> {
    match table.get(key) {
        Some(value) => value
            .as_array()
            .map(Vec::as_slice)
            .ok_or_else(|| format!("{CONFIG}: `{key}` must be an array of tables")),
        None => Ok(&[]),
    }
}

pub(super) fn text_value(table: &toml::Value, key: &str) -> Result<String, String> {
    table
        .get(key)
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{CONFIG}: table needs string `{key}`"))
}

pub(super) fn module_path(crate_path: &str, file: &str) -> Option<Vec<String>> {
    let rest = file.strip_prefix(&format!("{crate_path}/src/"))?;
    if rest.starts_with("bin/") {
        return None;
    }
    let stem = rest.strip_suffix(".rs")?;
    let mut parts: Vec<String> = stem.split('/').map(str::to_owned).collect();
    if parts.len() == 1 && matches!(parts.first().map(String::as_str), Some("lib" | "main")) {
        return Some(Vec::new());
    }
    if parts.last().is_some_and(|part| part == "mod") {
        parts.pop();
    }
    Some(parts)
}
