//! Module structure and dependency-direction gate for `docs/engineering/structure.md`.
//!
//! Tracks the mechanical parts of the structure rules: every source file states
//! its responsibility in a leading `//!` line, module and folder names use the
//! concept's name, folders stay narrow, and intra-crate paths follow the layer
//! table in `xtask/modules.toml`. Current violations live in
//! `xtask/modules-baseline.txt`, which only shrinks; the judgement calls in
//! rule 7 stay with the reviewer.
use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    fs,
    path::{Path, PathBuf},
};
use syn::{Attribute, Item, ItemMod, ItemUse, Path as SynPath, UseTree, Visibility, visit::Visit};

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
        if is_test_source(path) {
            continue;
        }
        violations.extend(leading_doc(root, path)?);
        violations.extend(file_name(path));
    }
    violations.extend(folder_rules(&tracked));
    violations.extend(imports(root, &config, &sources)?);
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

fn leading_doc(root: &Path, path: &str) -> Result<Option<Violation>, String> {
    let text = fs::read_to_string(root.join(path)).map_err(|e| format!("{path}: {e}"))?;
    let Some(first) = text.lines().next() else {
        return Err(format!("{path}: empty Rust source"));
    };
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

fn file_name(path: &str) -> Option<Violation> {
    let name = Path::new(path).file_stem()?.to_str()?;
    if name == "mod" || !BANNED.contains(&name) {
        return None;
    }
    Some(Violation {
        path: path.to_owned(),
        rule: NAME,
        detail: format!("module name `{name}` is banned"),
    })
}

fn folder_rules(tracked: &[String]) -> BTreeSet<Violation> {
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
            && BANNED.contains(&name)
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

fn is_source_dir(dir: &str) -> bool {
    dir == "xtask/src" || dir.ends_with("/src") || dir.contains("/src/")
}

#[derive(Clone)]
struct Placement {
    layer: usize,
    layer_name: String,
    group: String,
}

struct CrateTable {
    path: String,
    placements: BTreeMap<String, Placement>,
}

struct Table {
    crates: Vec<CrateTable>,
}

impl Table {
    fn crate_for(&self, file: &str) -> Option<&CrateTable> {
        self.crates
            .iter()
            .filter(|entry| {
                file.strip_prefix(&entry.path)
                    .is_some_and(|rest| rest.starts_with('/'))
            })
            .max_by_key(|entry| entry.path.len())
    }

    fn validate(&self, sources: &[&str]) -> Result<(), String> {
        for entry in &self.crates {
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
            }
        }
        Ok(())
    }
}

fn config(root: &Path) -> Result<Table, String> {
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

fn array<'a>(table: &'a toml::Value, key: &str) -> Result<&'a [toml::Value], String> {
    match table.get(key) {
        Some(value) => value
            .as_array()
            .map(Vec::as_slice)
            .ok_or_else(|| format!("{CONFIG}: `{key}` must be an array of tables")),
        None => Ok(&[]),
    }
}

fn text_value(table: &toml::Value, key: &str) -> Result<String, String> {
    table
        .get(key)
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{CONFIG}: table needs string `{key}`"))
}

fn imports(root: &Path, config: &Table, sources: &[&str]) -> Result<BTreeSet<Violation>, String> {
    let mut violations = BTreeSet::new();
    let mut exports: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
    for file in sources {
        if is_test_source(file) {
            continue;
        }
        let Some(crate_table) = config.crate_for(file) else {
            continue;
        };
        let Some(module) = module_path(&crate_table.path, file) else {
            continue;
        };
        let text = fs::read_to_string(root.join(file)).map_err(|e| format!("{file}: {e}"))?;
        let parsed = syn::parse_file(&text).map_err(|e| format!("{file}: invalid Rust: {e}"))?;
        let mut scan = Scan {
            module,
            uses: Vec::new(),
        };
        scan.visit_file(&parsed);
        for (at, path) in scan.uses {
            let Some(target) = resolve(&at, &path) else {
                continue;
            };
            let Some(target_top) = target.first() else {
                continue;
            };
            if let (Some(importer_top), Some(importer), Some(place)) = (
                at.first(),
                at.first().and_then(|top| crate_table.placements.get(top)),
                crate_table.placements.get(target_top),
            ) {
                if place.layer > importer.layer {
                    violations.insert(Violation {
                        path: file.to_string(),
                        rule: LAYER,
                        detail: format!(
                            "crate::{target_top} is in higher layer {}/{} (module {importer_top} is {}/{})",
                            place.layer_name, place.group, importer.layer_name, importer.group
                        ),
                    });
                } else if place.layer == importer.layer && place.group != importer.group {
                    violations.insert(Violation {
                        path: file.to_string(),
                        rule: LAYER,
                        detail: format!(
                            "crate::{target_top} crosses feature groups in layer {} (module {importer_top} is {}, target is {})",
                            place.layer_name, importer.group, place.group
                        ),
                    });
                }
            }
            if target.len() < 3 {
                continue;
            }
            let folder = root.join(&crate_table.path).join("src").join(target_top);
            if !folder.is_dir() {
                continue;
            }
            let interface = [folder.with_extension("rs"), folder.join("mod.rs")]
                .into_iter()
                .find(|candidate| candidate.is_file());
            let Some(interface) = interface else {
                continue;
            };
            if let Entry::Vacant(entry) = exports.entry(interface.clone()) {
                entry.insert(exported_modules(&interface)?);
            }
            let Some(exported) = exports.get(&interface) else {
                continue;
            };
            let Some(first) = target.get(1) else {
                continue;
            };
            if !exported.contains(first) && !exported.contains("*") {
                violations.insert(Violation {
                    path: file.to_string(),
                    rule: INTERFACE,
                    detail: format!(
                        "crate::{} reaches past the {} interface",
                        target.join("::"),
                        relative(root, &interface)
                    ),
                });
            }
        }
    }
    Ok(violations)
}

struct Scan {
    module: Vec<String>,
    uses: Vec<(Vec<String>, Vec<String>)>,
}

impl<'ast> Visit<'ast> for Scan {
    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        if is_cfg_test(&item.attrs) || item.content.is_none() {
            return;
        }
        self.module.push(item.ident.to_string());
        syn::visit::visit_item_mod(self, item);
        self.module.pop();
    }

    fn visit_path(&mut self, path: &'ast SynPath) {
        let segments: Vec<String> = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect();
        if path.leading_colon.is_none()
            && segments
                .first()
                .is_some_and(|first| matches!(first.as_str(), "crate" | "self" | "super"))
        {
            self.uses.push((self.module.clone(), segments));
        }
        syn::visit::visit_path(self, path);
    }

    fn visit_item_use(&mut self, item: &'ast ItemUse) {
        let mut paths = Vec::new();
        expand(&item.tree, &mut Vec::new(), &mut paths);
        for path in paths {
            if path
                .first()
                .is_some_and(|first| matches!(first.as_str(), "crate" | "self" | "super"))
            {
                self.uses.push((self.module.clone(), path));
            }
        }
    }
}

fn expand(tree: &UseTree, prefix: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            expand(&path.tree, prefix, out);
            prefix.pop();
        }
        UseTree::Name(name) => {
            let mut path = prefix.clone();
            path.push(name.ident.to_string());
            out.push(path);
        }
        UseTree::Rename(rename) => {
            let mut path = prefix.clone();
            path.push(rename.ident.to_string());
            out.push(path);
        }
        UseTree::Glob(_) => out.push(prefix.clone()),
        UseTree::Group(group) => {
            for item in &group.items {
                expand(item, prefix, out);
            }
        }
    }
}

fn resolve(module: &[String], path: &[String]) -> Option<Vec<String>> {
    let mut base = module.to_vec();
    let mut segments = path.iter();
    match segments.next()?.as_str() {
        "crate" => base.clear(),
        "self" => {}
        "super" => {
            base.pop()?;
        }
        _ => return None,
    }
    for segment in segments {
        match segment.as_str() {
            "self" => {}
            "super" => {
                base.pop()?;
            }
            "crate" => return None,
            _ => base.push(segment.clone()),
        }
    }
    Some(base)
}

fn exported_modules(path: &Path) -> Result<BTreeSet<String>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file =
        syn::parse_file(&text).map_err(|e| format!("{}: invalid Rust: {e}", path.display()))?;
    let mut exported = BTreeSet::new();
    for item in &file.items {
        match item {
            Item::Mod(item) if !private(&item.vis) => {
                exported.insert(item.ident.to_string());
            }
            Item::Use(item) if !private(&item.vis) => {
                let mut names = Vec::new();
                use_names(&item.tree, &mut names);
                exported.extend(names);
            }
            Item::Const(item) if !private(&item.vis) => {
                exported.insert(item.ident.to_string());
            }
            Item::Enum(item) if !private(&item.vis) => {
                exported.insert(item.ident.to_string());
            }
            Item::Fn(item) if !private(&item.vis) => {
                exported.insert(item.sig.ident.to_string());
            }
            Item::Static(item) if !private(&item.vis) => {
                exported.insert(item.ident.to_string());
            }
            Item::Struct(item) if !private(&item.vis) => {
                exported.insert(item.ident.to_string());
            }
            Item::Trait(item) if !private(&item.vis) => {
                exported.insert(item.ident.to_string());
            }
            Item::TraitAlias(item) if !private(&item.vis) => {
                exported.insert(item.ident.to_string());
            }
            Item::Type(item) if !private(&item.vis) => {
                exported.insert(item.ident.to_string());
            }
            Item::Union(item) if !private(&item.vis) => {
                exported.insert(item.ident.to_string());
            }
            _ => {}
        }
    }
    Ok(exported)
}

fn use_names(tree: &UseTree, out: &mut Vec<String>) {
    match tree {
        UseTree::Path(path) => use_names(&path.tree, out),
        UseTree::Name(name) => out.push(name.ident.to_string()),
        UseTree::Rename(rename) => out.push(rename.rename.to_string()),
        UseTree::Glob(_) => out.push("*".into()),
        UseTree::Group(group) => {
            for item in &group.items {
                use_names(item, out);
            }
        }
    }
}

fn private(visibility: &Visibility) -> bool {
    matches!(visibility, Visibility::Inherited)
}

fn is_cfg_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("cfg") {
            return false;
        }
        let syn::Meta::List(list) = &attr.meta else {
            return false;
        };
        list.tokens.to_string().contains("test")
    })
}

fn module_path(crate_path: &str, file: &str) -> Option<Vec<String>> {
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
    let out = crate::checked(
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

fn baseline(root: &Path) -> Result<BTreeSet<Violation>, String> {
    let path = root.join(BASELINE);
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut lines = Vec::new();
    let mut entries = BTreeSet::new();
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let number = index + 1;
        let mut fields = line.splitn(3, '\t');
        let rule = fields
            .next()
            .and_then(rule_id)
            .ok_or_else(|| format!("{BASELINE}:{number}: unknown rule"))?;
        let file = fields
            .next()
            .ok_or_else(|| format!("{BASELINE}:{number}: expected `rule<TAB>path<TAB>detail`"))?;
        let detail = fields
            .next()
            .ok_or_else(|| format!("{BASELINE}:{number}: expected `rule<TAB>path<TAB>detail`"))?;
        if detail.contains('\t') {
            return Err(format!(
                "{BASELINE}:{number}: detail must not contain a tab"
            ));
        }
        entries.insert(Violation {
            path: file.to_owned(),
            rule,
            detail: detail.to_owned(),
        });
        lines.push(line.to_owned());
    }
    let mut sorted = lines.clone();
    sorted.sort();
    sorted.dedup();
    if lines != sorted {
        return Err(format!(
            "{BASELINE}: entries must be sorted by rule, path and detail without duplicates"
        ));
    }
    Ok(entries)
}

fn rule_id(rule: &str) -> Option<&'static str> {
    match rule {
        DOC => Some(DOC),
        NAME => Some(NAME),
        FOLDER => Some(FOLDER),
        INTERFACE => Some(INTERFACE),
        LAYER => Some(LAYER),
        _ => None,
    }
}

fn rule_number(rule: &str) -> u8 {
    match rule {
        DOC | NAME => 1,
        FOLDER => 2,
        INTERFACE => 3,
        LAYER => 4,
        _ => 0,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use std::{
        process::Command,
        sync::atomic::{AtomicUsize, Ordering},
    };

    struct Tree(PathBuf);

    impl Tree {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "vibecut-modules-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(dir.join("xtask")).unwrap();
            fs::write(dir.join(CONFIG), "").unwrap();
            fs::write(dir.join(BASELINE), "").unwrap();
            git(&dir, &["init", "--quiet"]);
            Self(dir)
        }

        fn write(&self, path: &str, text: &str) {
            let file = self.0.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
            git(&self.0, &["-c", "core.autocrlf=false", "add", "--", path]);
        }

        fn config(&self, text: &str) {
            fs::write(self.0.join(CONFIG), text).unwrap();
        }

        fn baseline(&self, rows: &[(&str, &str, &str)]) {
            let mut lines: Vec<String> = rows
                .iter()
                .map(|(rule, path, detail)| format!("{rule}\t{path}\t{detail}"))
                .collect();
            lines.sort();
            fs::write(self.0.join(BASELINE), format!("{}\n", lines.join("\n"))).unwrap();
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn git(root: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    const LAYERS: &str = r#"[[crate]]
name = "foo"
path = "crates/foo"

[[crate.layer]]
name = "base"
groups = ["base"]

[[crate.layer]]
name = "features"
groups = ["one", "two"]

[[crate.module]]
name = "low"
group = "base"

[[crate.module]]
name = "one"
group = "one"

[[crate.module]]
name = "two"
group = "two"

[[crate.module]]
name = "high"
group = "two"
"#;

    #[test]
    fn module_files_need_a_responsibility_line() {
        let tree = Tree::new();
        tree.write("crates/foo/src/lib.rs", "//! Foo.\npub mod thing;\n");
        tree.write("crates/foo/src/thing.rs", "//! Thing.\n");
        check(&tree.0).unwrap();
        tree.write("crates/foo/src/thing.rs", "pub struct Thing;\n");
        let error = check(&tree.0).unwrap_err();
        for expected in [
            "crates/foo/src/thing.rs",
            "[doc]",
            "structure rule 1",
            "first line is not a //!",
        ] {
            assert!(error.contains(expected), "{expected}: {error}");
        }
        tree.write("crates/foo/src/thing.rs", "//!\n");
        assert!(
            check(&tree.0)
                .unwrap_err()
                .contains("responsibility line is empty")
        );
    }

    #[test]
    fn banned_module_and_folder_names_fail() {
        let tree = Tree::new();
        tree.write("crates/foo/src/lib.rs", "//! Foo.\n");
        tree.write("crates/foo/src/utils.rs", "//! Helpers.\n");
        tree.write("crates/foo/src/helpers/thing.rs", "//! Thing.\n");
        let error = check(&tree.0).unwrap_err();
        for expected in [
            "crates/foo/src/utils.rs",
            "crates/foo/src/helpers",
            "[name]",
            "structure rule 1",
            "`utils` is banned",
            "`helpers` is banned",
        ] {
            assert!(error.contains(expected), "{expected}: {error}");
        }
    }

    #[test]
    fn folders_are_limited_to_twelve_children() {
        let tree = Tree::new();
        tree.write("crates/foo/src/lib.rs", "//! Foo.\n");
        for index in 0..12 {
            tree.write(
                &format!("crates/foo/src/wide/part{index}.rs"),
                "//! Part.\n",
            );
        }
        check(&tree.0).unwrap();
        tree.write("crates/foo/src/wide/part12.rs", "//! Part.\n");
        let error = check(&tree.0).unwrap_err();
        for expected in [
            "crates/foo/src/wide",
            "[folder]",
            "13 direct children (limit 12)",
            "structure rule 2",
        ] {
            assert!(error.contains(expected), "{expected}: {error}");
        }
    }

    #[test]
    fn imports_follow_layers_and_feature_groups() {
        let tree = Tree::new();
        tree.config(LAYERS);
        tree.write(
            "crates/foo/src/lib.rs",
            "//! Foo.\npub mod low;\npub mod one;\npub mod two;\npub mod high;\n",
        );
        tree.write(
            "crates/foo/src/low.rs",
            "//! Low.\nuse crate::high::High;\n",
        );
        tree.write("crates/foo/src/one.rs", "//! One.\npub struct One;\n");
        tree.write("crates/foo/src/two.rs", "//! Two.\npub struct Two;\n");
        tree.write("crates/foo/src/high.rs", "//! High.\npub struct High;\n");
        let error = check(&tree.0).unwrap_err();
        for expected in [
            "crates/foo/src/low.rs",
            "[layer]",
            "structure rule 4",
            "higher layer",
        ] {
            assert!(error.contains(expected), "{expected}: {error}");
        }
        tree.write("crates/foo/src/low.rs", "//! Low.\npub struct Low;\n");
        tree.write(
            "crates/foo/src/high.rs",
            "//! High.\nuse crate::low::Low;\npub struct High;\n",
        );
        check(&tree.0).unwrap();
        tree.write("crates/foo/src/one.rs", "//! One.\nuse crate::two::Two;\n");
        let error = check(&tree.0).unwrap_err();
        assert!(error.contains("crosses feature groups"), "{error}");
        tree.write("crates/foo/src/one.rs", "//! One.\nuse crate::low::Low;\n");
        check(&tree.0).unwrap();
    }

    #[test]
    fn imports_may_not_reach_past_a_folder_interface() {
        let tree = Tree::new();
        tree.config(
            r#"[[crate]]
name = "foo"
path = "crates/foo"

[[crate.layer]]
name = "base"
groups = ["base"]

[[crate.module]]
name = "foo"
group = "base"

[[crate.module]]
name = "user"
group = "base"
"#,
        );
        tree.write(
            "crates/foo/src/lib.rs",
            "//! Foo.\npub mod foo;\npub mod user;\n",
        );
        tree.write(
            "crates/foo/src/foo.rs",
            "//! Foo.\nmod bar;\npub use bar::Thing;\n",
        );
        tree.write("crates/foo/src/foo/bar.rs", "//! Bar.\npub struct Thing;\n");
        tree.write(
            "crates/foo/src/user.rs",
            "//! User.\nuse crate::foo::bar::Thing;\n",
        );
        let error = check(&tree.0).unwrap_err();
        for expected in [
            "crates/foo/src/user.rs",
            "[interface]",
            "structure rule 3",
            "reaches past",
            "crates/foo/src/foo.rs",
        ] {
            assert!(error.contains(expected), "{expected}: {error}");
        }
        tree.write(
            "crates/foo/src/user.rs",
            "//! User.\nuse crate::foo::Thing;\n",
        );
        check(&tree.0).unwrap();
        tree.write("crates/foo/src/foo.rs", "//! Foo.\npub mod bar;\n");
        tree.write(
            "crates/foo/src/user.rs",
            "//! User.\nuse crate::foo::bar::Thing;\n",
        );
        check(&tree.0).unwrap();
    }

    #[test]
    fn baselined_violations_pass_and_new_ones_fail() {
        let tree = Tree::new();
        tree.write("crates/foo/src/lib.rs", "//! Foo.\n");
        tree.write("crates/foo/src/thing.rs", "pub struct Thing;\n");
        assert!(check(&tree.0).unwrap_err().contains("[doc]"));
        tree.baseline(&[(
            "doc",
            "crates/foo/src/thing.rs",
            "first line is not a //! responsibility line",
        )]);
        check(&tree.0).unwrap();
        tree.write("crates/foo/src/other.rs", "pub struct Other;\n");
        let error = check(&tree.0).unwrap_err();
        assert!(error.contains("crates/foo/src/other.rs"), "{error}");
        assert!(error.contains("[doc]"), "{error}");
        assert!(!error.contains("no longer occurs"), "{error}");
    }

    #[test]
    fn baseline_entries_that_no_longer_occur_fail() {
        let tree = Tree::new();
        tree.write("crates/foo/src/lib.rs", "//! Foo.\n");
        tree.write("crates/foo/src/thing.rs", "pub struct Thing;\n");
        tree.baseline(&[(
            "doc",
            "crates/foo/src/thing.rs",
            "first line is not a //! responsibility line",
        )]);
        check(&tree.0).unwrap();
        tree.write("crates/foo/src/thing.rs", "//! Thing.\n");
        let error = check(&tree.0).unwrap_err();
        assert!(error.contains("no longer occurs"), "{error}");
        assert!(error.contains("remove it"), "{error}");
    }

    #[test]
    fn output_is_deterministic_and_sorted() {
        let tree = Tree::new();
        tree.write("crates/foo/src/lib.rs", "//! Foo.\n");
        tree.write("crates/foo/src/zulu.rs", "pub struct Zulu;\n");
        tree.write("crates/foo/src/alpha.rs", "pub struct Alpha;\n");
        tree.write("crates/foo/src/utils.rs", "//! Helpers.\n");
        let first = check(&tree.0).unwrap_err();
        assert_eq!(first, check(&tree.0).unwrap_err());
        let lines: Vec<&str> = first.lines().collect();
        let mut sorted = lines.clone();
        sorted.sort();
        assert_eq!(lines, sorted);
    }

    #[test]
    fn crates_without_a_table_get_only_structural_checks() {
        let tree = Tree::new();
        tree.write(
            "crates/foo/src/lib.rs",
            "//! Foo.\npub mod low;\npub mod high;\n",
        );
        tree.write(
            "crates/foo/src/low.rs",
            "//! Low.\nuse crate::high::High;\n",
        );
        tree.write("crates/foo/src/high.rs", "//! High.\npub struct High;\n");
        check(&tree.0).unwrap();
    }
}
