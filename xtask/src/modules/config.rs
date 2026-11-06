//! The `xtask/modules.toml` layer table and the module paths it applies to.
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

pub(super) struct CrateTable {
    pub(super) path: String,
    pub(super) placements: BTreeMap<String, Placement>,
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
        }
        Ok(())
    }
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
        crates.push(CrateTable { path, placements });
    }
    Ok(Table { crates })
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
