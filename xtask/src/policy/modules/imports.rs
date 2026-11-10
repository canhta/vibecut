//! Intra-crate import direction and folder-interface rules over parsed Rust.
use super::{
    INTERFACE, LAYER, Violation,
    config::{CrateTable, FolderTable, Placement, Table, inside, module_path},
    is_test_source, relative,
    syntax::{is_cfg_test, item_attrs},
};
use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    fs,
    path::{Path, PathBuf},
};
use syn::{ImplItemFn, Item, ItemMod, ItemUse, Path as SynPath, UseTree, Visibility, visit::Visit};

const CONTROL_CATALOG: &str = "crates/engine/tests/it/control/catalog.rs";
const CONTROL_IMPORT: &str = "use crate::support::control::";

/// `control/catalog.rs` imports one support name per line in ascending order,
/// so two appended helpers never share a line.
pub(super) fn control_test_order(root: &Path) -> Result<(), String> {
    let path = root.join(CONTROL_CATALOG);
    let text = fs::read_to_string(&path).map_err(|e| format!("{CONTROL_CATALOG}: {e}"))?;
    control_order(&text)?;
    eprintln!("control imports: ok");
    Ok(())
}

pub(super) fn control_order(text: &str) -> Result<(), String> {
    let mut previous: Option<(&str, usize)> = None;
    for (index, line) in text.lines().enumerate() {
        let Some(rest) = line.trim_start().strip_prefix(CONTROL_IMPORT) else {
            continue;
        };
        let name = rest.trim_end_matches(';').trim();
        if name.is_empty() || name.contains('{') || name.contains(',') || name.contains(' ') {
            return Err(format!(
                "{CONTROL_CATALOG}:{}: control support imports must be one name per line; split `{name}`",
                index + 1
            ));
        }
        if let Some((before, before_line)) = previous
            && before > name
        {
            return Err(format!(
                "{CONTROL_CATALOG}:{}: control support import `{name}` sorts before `{before}` (line {before_line}); keep imports in ascending order",
                index + 1
            ));
        }
        previous = Some((name, index + 1));
    }
    if previous.is_none() {
        return Err(format!(
            "{CONTROL_CATALOG}: no `{CONTROL_IMPORT}...` imports found"
        ));
    }
    Ok(())
}

pub(super) fn imports(
    root: &Path,
    config: &Table,
    sources: &[&str],
) -> Result<BTreeSet<Violation>, String> {
    let mut violations = BTreeSet::new();
    let mut exports: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
    let mut reexports: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
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
        if !reexports.contains_key(&crate_table.path) {
            reexports.insert(crate_table.path.clone(), root_reexports(root, crate_table)?);
        }
        let Some(root_names) = reexports.get(&crate_table.path) else {
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
            let Some(first) = target.first() else {
                continue;
            };
            // `crate::Name` can be a `lib.rs` re-export of a module's item.
            let reexported = !crate_table.placements.contains_key(first);
            let target_top = if reexported {
                match root_names.get(first) {
                    Some(module) => module,
                    None => continue,
                }
            } else {
                first
            };
            let importer_top = at.first();
            if let (Some(importer_top), Some(importer), Some(place)) = (
                importer_top,
                importer_top.and_then(|top| crate_table.placements.get(top)),
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
            if let Some((folder, importer, place)) = folder_placements(crate_table, &at, &target) {
                let importer_name = at
                    .get(..=folder.prefix.len())
                    .unwrap_or_default()
                    .join("::");
                let target_name = target
                    .get(..=folder.prefix.len())
                    .unwrap_or_default()
                    .join("::");
                if place.layer > importer.layer {
                    violations.insert(Violation {
                        path: file.to_string(),
                        rule: LAYER,
                        detail: format!(
                            "crate::{target_name} is in higher layer {}/{} inside folder `{}` (module {importer_name} is {}/{})",
                            place.layer_name,
                            place.group,
                            folder.path,
                            importer.layer_name,
                            importer.group
                        ),
                    });
                } else if place.layer == importer.layer && place.group != importer.group {
                    violations.insert(Violation {
                        path: file.to_string(),
                        rule: LAYER,
                        detail: format!(
                            "crate::{target_name} crosses feature groups in layer {} inside folder `{}` (module {importer_name} is {}, target is {})",
                            place.layer_name, folder.path, importer.group, place.group
                        ),
                    });
                }
            }
            // A folder's own children may use each other; only outsiders go through its door.
            if reexported || importer_top == Some(target_top) {
                continue;
            }
            let Some(child) = target.get(1) else {
                continue;
            };
            let folder = root.join(&crate_table.path).join("src").join(target_top);
            if !folder.is_dir()
                || !(folder.join(child).is_dir() || folder.join(format!("{child}.rs")).is_file())
            {
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
            if !exported.contains(child) {
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

/// The deepest folder table that places both the importer and the target module.
fn folder_placements<'a>(
    crate_table: &'a CrateTable,
    importer: &[String],
    target: &[String],
) -> Option<(&'a FolderTable, &'a Placement, &'a Placement)> {
    crate_table
        .folders
        .iter()
        .filter(|folder| inside(folder, importer) && inside(folder, target))
        .max_by_key(|folder| folder.prefix.len())
        .and_then(|folder| {
            let importer = folder.placements.get(importer.get(folder.prefix.len())?)?;
            let target = folder.placements.get(target.get(folder.prefix.len())?)?;
            Some((folder, importer, target))
        })
}

/// Names the crate root re-exports from its top-level modules, mapped to the module.
pub(super) fn root_reexports(
    root: &Path,
    crate_table: &CrateTable,
) -> Result<BTreeMap<String, String>, String> {
    let src = root.join(&crate_table.path).join("src");
    let Some(entry) = ["lib.rs", "main.rs"]
        .into_iter()
        .map(|name| src.join(name))
        .find(|candidate| candidate.is_file())
    else {
        return Ok(BTreeMap::new());
    };
    let text = fs::read_to_string(&entry).map_err(|e| format!("{}: {e}", entry.display()))?;
    let file =
        syn::parse_file(&text).map_err(|e| format!("{}: invalid Rust: {e}", entry.display()))?;
    let mut bindings = Vec::new();
    for item in &file.items {
        if let Item::Use(item) = item {
            expand(&item.tree, &mut Vec::new(), &mut bindings);
        }
    }
    // `use editor as frontend;` renames a module; `pub use frontend::Editor;` re-exports through it.
    let mut aliases = BTreeMap::new();
    for (path, name) in &bindings {
        if let [module] = path.as_slice()
            && module != name
            && crate_table.placements.contains_key(module)
        {
            aliases.insert(name.clone(), module.clone());
        }
    }
    let mut names = aliases.clone();
    for (path, name) in bindings {
        let mut segments = path.iter().map(String::as_str);
        let mut head = segments.next();
        if matches!(head, Some("crate" | "self")) {
            head = segments.next();
        }
        let Some(head) = head else { continue };
        let module = aliases.get(head).map_or(head, String::as_str);
        if crate_table.placements.contains_key(module) && name != "*" && path.len() > 1 {
            names.insert(name, module.to_owned());
        }
    }
    Ok(names)
}

pub(super) struct Scan {
    module: Vec<String>,
    uses: Vec<(Vec<String>, Vec<String>)>,
}

impl<'ast> Visit<'ast> for Scan {
    fn visit_item(&mut self, item: &'ast Item) {
        if !is_cfg_test(item_attrs(item)) {
            syn::visit::visit_item(self, item);
        }
    }

    fn visit_impl_item_fn(&mut self, item: &'ast ImplItemFn) {
        if !is_cfg_test(&item.attrs) {
            syn::visit::visit_impl_item_fn(self, item);
        }
    }

    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        if item.content.is_none() {
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
        for (path, _) in paths {
            if path
                .first()
                .is_some_and(|first| matches!(first.as_str(), "crate" | "self" | "super"))
            {
                self.uses.push((self.module.clone(), path));
            }
        }
    }
}

/// Each `use` leaf as (full path, bound name); a glob binds `*`.
pub(super) fn expand(
    tree: &UseTree,
    prefix: &mut Vec<String>,
    out: &mut Vec<(Vec<String>, String)>,
) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            expand(&path.tree, prefix, out);
            prefix.pop();
        }
        UseTree::Name(name) => {
            let mut path = prefix.clone();
            path.push(name.ident.to_string());
            out.push((path, name.ident.to_string()));
        }
        UseTree::Rename(rename) => {
            let mut path = prefix.clone();
            path.push(rename.ident.to_string());
            out.push((path, rename.rename.to_string()));
        }
        UseTree::Glob(_) => out.push((prefix.clone(), "*".into())),
        UseTree::Group(group) => {
            for item in &group.items {
                expand(item, prefix, out);
            }
        }
    }
}

pub(super) fn resolve(module: &[String], path: &[String]) -> Option<Vec<String>> {
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

pub(super) fn exported_modules(path: &Path) -> Result<BTreeSet<String>, String> {
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

pub(super) fn use_names(tree: &UseTree, out: &mut Vec<String>) {
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

pub(super) fn private(visibility: &Visibility) -> bool {
    matches!(visibility, Visibility::Inherited)
}
