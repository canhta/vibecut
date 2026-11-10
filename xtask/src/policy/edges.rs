//! Crate-edge policy: the architecture table admits packages, dependency edges and test layout.
use crate::process::checked;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

pub(crate) struct Rule {
    pub(crate) path: String,
    pub(crate) allowed: Vec<String>,
    pub(crate) dev_only: bool,
}

pub(crate) fn policy(root: &Path) -> Result<BTreeMap<String, Rule>, String> {
    let doc = fs::read_to_string(root.join("docs/engineering/architecture.md"))
        .map_err(|e| e.to_string())?;
    let mut rules = BTreeMap::new();
    for line in doc.lines().filter(|s| s.starts_with("| ")) {
        let cells: Vec<_> = line.split('|').map(str::trim).collect();
        if let (Some(name), Some(path), Some(deps)) = (cells.get(1), cells.get(2), cells.get(4))
            && (path.starts_with("crates/") || path.starts_with("apps/"))
        {
            let allowed = deps
                .split(',')
                .map(str::trim)
                .filter(|d| *d != "-")
                .map(str::to_owned)
                .collect();
            let dev_only = cells
                .get(3)
                .is_some_and(|owns| owns.to_ascii_lowercase().starts_with("dev-only"));
            if rules
                .insert(
                    (*name).into(),
                    Rule {
                        path: (*path).into(),
                        allowed,
                        dev_only,
                    },
                )
                .is_some()
            {
                return Err(format!("duplicate architecture package: {name}"));
            }
        }
    }
    if rules.is_empty() {
        return Err("architecture crate table is empty".into());
    }
    Ok(rules)
}

/// Crates only apps may use, grouped by the capability they bring in. Add a
/// crate only with a named consumer; the architecture table stays the internal
/// edge authority.
const APP_ONLY_CRATES: &[(&str, &str)] = &[
    ("accesskit", "accessibility"), // apps/vibecut, ticket 11
    ("cpal", "audio-device"),       // apps/vibecut, ticket 11
    ("eframe", "ui"),               // apps/vibecut, ticket 11
    ("egui", "ui"),                 // apps/vibecut, ticket 11
    ("egui_kittest", "ui"),         // apps/vibecut, ticket 11
    ("native-dialog", "window"),    // apps/vibecut file dialogs, ticket 13
    ("rfd", "window"),              // apps/vibecut file dialogs, ticket 13
    ("wgpu", "gpu"),                // apps/vibecut adapter, ticket 11
    ("winit", "window"),            // apps/vibecut, ticket 11
];

fn forbidden_app_only(package: &str, dependency: &Value) -> Option<String> {
    let name = dependency["name"].as_str()?;
    let class = APP_ONLY_CRATES
        .iter()
        .find_map(|(crate_name, class)| (*crate_name == name).then_some(*class))?;
    let kind = dependency["kind"].as_str().unwrap_or("normal");
    let mut detail = format!("kind {kind}");
    if let Some(target) = dependency["target"].as_str() {
        detail.push_str(&format!(", target {target}"));
    }
    if let Some(rename) = dependency["rename"].as_str() {
        detail.push_str(&format!(", renamed {rename}"));
    }
    Some(format!(
        "forbidden app-only {class} crate: package `{package}` dependency `{name}` ({detail})"
    ))
}

/// Inspect Cargo's actual targets, including explicit paths and disabled harnesses.
fn integration_test_layout(package: &Value) -> Result<(), String> {
    let manifest = Path::new(
        package["manifest_path"]
            .as_str()
            .ok_or("missing manifest path")?,
    );
    let expected = manifest
        .parent()
        .ok_or("manifest has no parent")?
        .join("tests/it/main.rs");
    let targets = package["targets"]
        .as_array()
        .ok_or("package has no targets")?;
    let mut count = 0;
    for target in targets {
        if !target["kind"]
            .as_array()
            .is_some_and(|kinds| kinds.iter().any(|kind| kind == "test"))
        {
            continue;
        }
        count += 1;
        let path = Path::new(
            target["src_path"]
                .as_str()
                .ok_or("test target has no source path")?,
        );
        if path != expected || count > 1 {
            return Err(format!(
                "{}: move integration tests to tests/it/ as modules of the single tests/it/main.rs binary",
                path.display()
            ));
        }
    }
    Ok(())
}

pub(crate) fn layers(root: &Path) -> Result<(), String> {
    let rules = policy(root)?;
    let bytes = checked(
        root,
        env!("CARGO"),
        &[
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
        ],
    )?;
    let md: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let packages = md
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("metadata has no packages")?;
    let members = md
        .get("workspace_members")
        .and_then(Value::as_array)
        .ok_or("metadata has no members")?;
    let mut errors = Vec::new();
    let dev_only: BTreeSet<&str> = rules
        .iter()
        .filter(|(_, rule)| rule.dev_only)
        .map(|(name, _)| name.as_str())
        .collect();
    for p in packages.iter().filter(|p| members.contains(&p["id"])) {
        let name = p["name"].as_str().ok_or("package has no name")?;
        integration_test_layout(p)?;
        let deps = p["dependencies"]
            .as_array()
            .ok_or("package has no dependency array")?;
        let (path, allowed) = if name == "xtask" {
            ("xtask".to_owned(), Vec::new())
        } else {
            let rule = rules
                .get(name)
                .ok_or_else(|| format!("unadmitted package: {name}"))?;
            let manifest = Path::new(p["manifest_path"].as_str().ok_or("missing manifest path")?);
            if manifest != root.join(&rule.path).join("Cargo.toml") {
                errors.push(format!("{name}: expected {}/Cargo.toml", rule.path));
            }
            (rule.path.clone(), rule.allowed.clone())
        };
        if !path.starts_with("apps/") {
            errors.extend(deps.iter().filter_map(|d| forbidden_app_only(name, d)));
        }
        for d in deps {
            let dep = d["name"].as_str().ok_or("dependency has no name")?;
            // Cargo reports the package name even when a dependency is renamed.
            let internal = rules.contains_key(dep) && d["path"].is_string()
                || packages.iter().any(|p| {
                    p["name"] == dep && members.contains(&p["id"]) && d["path"].is_string()
                });
            if internal && !allowed.iter().any(|a| a == dep) {
                errors.push(format!("forbidden workspace edge: {name} -> {dep}"));
            }
            if internal && dev_only.contains(dep) {
                let kind = d["kind"].as_str().unwrap_or("normal");
                if kind != "dev" {
                    errors.push(format!(
                        "{name} -> {dep}: dev-only crate must be a [dev-dependencies] entry (kind {kind})"
                    ));
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    eprintln!("layers: ok");
    Ok(())
}
