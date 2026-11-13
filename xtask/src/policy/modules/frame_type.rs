//! Rule 5: modules of the listed layers never name a type the frame alone owns.
use super::{
    FRAME_TYPE, Violation,
    config::{Table, inside, module_path},
    is_test_source,
    syntax::{is_cfg_test, item_attrs},
};
use std::{collections::BTreeSet, fs, path::Path};
use syn::{ImplItemFn, Item, visit::Visit};

pub(super) fn frame_types(
    root: &Path,
    config: &Table,
    sources: &[&str],
) -> Result<BTreeSet<Violation>, String> {
    let mut violations = BTreeSet::new();
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
        let mut names: Vec<&str> = Vec::new();
        for folder in crate_table.folders.iter().filter(|f| inside(f, &module)) {
            let Some(place) = module
                .get(folder.prefix.len())
                .and_then(|child| folder.placements.get(child))
            else {
                continue;
            };
            for rule in &folder.forbidden {
                if rule.layers.contains(&place.layer_name) {
                    names.push(&rule.name);
                }
            }
        }
        if names.is_empty() {
            continue;
        }
        let text = fs::read_to_string(root.join(file)).map_err(|e| format!("{file}: {e}"))?;
        let parsed = syn::parse_file(&text).map_err(|e| format!("{file}: invalid Rust: {e}"))?;
        let mut scan = Idents(BTreeSet::new());
        scan.visit_file(&parsed);
        for name in names {
            if scan.0.contains(name) {
                violations.insert(Violation {
                    path: (*file).to_owned(),
                    rule: FRAME_TYPE,
                    detail: format!("names `{name}`, the frame record; take a port instead"),
                });
            }
        }
    }
    Ok(violations)
}

/// Every identifier outside test-only items; comments and strings are not identifiers.
struct Idents(BTreeSet<String>);

impl<'ast> Visit<'ast> for Idents {
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

    fn visit_ident(&mut self, ident: &'ast syn::Ident) {
        self.0.insert(ident.to_string());
    }
}
