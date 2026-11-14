//! The shipped Rust crate list: generated from `cargo metadata`, committed and checked for drift.
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

/// The committed list the About dialog reads: one `name<TAB>version<TAB>licence` line per crate.
pub(crate) const FILE: &str = "apps/vibecut/third-party-crates.txt";

/// The shipped platforms, as in `deny.toml`.
const TARGETS: [&str; 6] = [
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
];

/// The package whose normal-dependency closure ships.
const APP: &str = "vibecut";

/// Stands in for a crate that declares no licence expression.
const UNDECLARED: &str = "see crate";

type Row = (String, String, String);

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// Crates outside the workspace reachable from `app` through normal dependencies.
fn closure(metadata: &Value, app: &str) -> Result<BTreeSet<Row>, String> {
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("metadata has no packages")?;
    let nodes = metadata
        .pointer("/resolve/nodes")
        .and_then(Value::as_array)
        .ok_or("metadata has no resolve graph")?;
    let workspace = |package: &Value| package.get("source").is_none_or(Value::is_null);
    let start = packages
        .iter()
        .find(|package| text(package, "name") == Some(app) && workspace(package))
        .and_then(|package| text(package, "id"))
        .ok_or_else(|| format!("no workspace package `{app}`"))?;
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut pending = vec![start];
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(node) = nodes.iter().find(|node| text(node, "id") == Some(id)) else {
            continue;
        };
        for dep in node
            .get("deps")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let normal = dep
                .get("dep_kinds")
                .and_then(Value::as_array)
                .is_some_and(|kinds| {
                    kinds
                        .iter()
                        .any(|kind| kind.get("kind").is_some_and(Value::is_null))
                });
            if let (true, Some(package)) = (normal, text(dep, "pkg")) {
                pending.push(package);
            }
        }
    }
    let mut rows = BTreeSet::new();
    for package in packages {
        let Some(id) = text(package, "id") else {
            continue;
        };
        if !seen.contains(id) || workspace(package) {
            continue;
        }
        let (Some(name), Some(version)) = (text(package, "name"), text(package, "version")) else {
            continue;
        };
        let licence = text(package, "license").unwrap_or(UNDECLARED);
        rows.insert((name.to_owned(), version.to_owned(), licence.to_owned()));
    }
    Ok(rows)
}

fn render(rows: &BTreeSet<Row>) -> String {
    rows.iter()
        .map(|(name, version, licence)| format!("{name}\t{version}\t{licence}\n"))
        .collect()
}

/// The list as `cargo metadata --locked` resolves it for every shipped platform.
fn generate(root: &Path) -> Result<String, String> {
    let mut rows = BTreeSet::new();
    for target in TARGETS {
        let output = Command::new(env!("CARGO"))
            .args([
                "metadata",
                "--locked",
                "--format-version",
                "1",
                "--filter-platform",
                target,
            ])
            .current_dir(root)
            .output()
            .map_err(|e| format!("cargo metadata: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "cargo metadata for {target} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let metadata: Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| format!("cargo metadata for {target}: {e}"))?;
        rows.extend(closure(&metadata, APP)?);
    }
    Ok(render(&rows))
}

/// `cargo xtask licences`: rewrite the committed list.
pub(crate) fn write(root: &Path) -> Result<(), String> {
    let path = root.join(FILE);
    fs::write(&path, generate(root)?).map_err(|e| format!("{}: {e}", path.display()))
}

/// Fail when the committed list differs from what the lockfile resolves to.
pub(crate) fn check(root: &Path) -> Result<(), String> {
    let path = root.join(FILE);
    let committed = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if committed.replace("\r\n", "\n") != generate(root)? {
        return Err(format!("{FILE} is out of date; run `cargo xtask licences`"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn package(name: &str, source: Option<&str>, license: Option<&str>) -> Value {
        json!({
            "name": name,
            "version": "1.0.0",
            "id": name,
            "source": source,
            "license": license,
        })
    }

    fn node(id: &str, deps: &[(&str, Option<&str>)]) -> Value {
        let deps: Vec<Value> = deps
            .iter()
            .map(|(pkg, kind)| json!({ "pkg": pkg, "dep_kinds": [{ "kind": kind, "target": null }] }))
            .collect();
        json!({ "id": id, "deps": deps })
    }

    #[test]
    fn closure_keeps_shipped_external_crates_with_their_licences() {
        let metadata = json!({
            "packages": [
                package("vibecut", None, Some("GPL-3.0-only")),
                package("engine", None, Some("GPL-3.0-only")),
                package("a", Some("registry"), Some("MIT OR Apache-2.0")),
                package("b", Some("registry"), None),
                package("dev-only", Some("registry"), Some("MIT")),
                package("build-only", Some("registry"), Some("MIT")),
                package("unreached", Some("registry"), Some("MIT")),
            ],
            "resolve": { "nodes": [
                node("vibecut", &[
                    ("engine", None),
                    ("a", None),
                    ("dev-only", Some("dev")),
                    ("build-only", Some("build")),
                ]),
                node("engine", &[("b", None)]),
                node("a", &[("b", None)]),
                node("b", &[]),
            ] },
        });
        let rows = closure(&metadata, "vibecut").unwrap_or_default();
        assert_eq!(
            render(&rows),
            "a\t1.0.0\tMIT OR Apache-2.0\nb\t1.0.0\tsee crate\n"
        );
    }

    #[test]
    fn closure_refuses_metadata_without_the_app() {
        let metadata = json!({ "packages": [], "resolve": { "nodes": [] } });
        assert!(closure(&metadata, "vibecut").is_err());
    }
}
