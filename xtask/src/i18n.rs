//! Catalog completeness and UI-literal audit for `apps/vibecut`.
//!
//! The app's `Text` table is compiler-checked at call sites; this check covers
//! what the compiler cannot see: empty or duplicate rows, keys no source file
//! uses, and direct string literals passed to visible egui widgets. Rust is
//! parsed as syntax, so comments, strings, multiline arguments and raw strings
//! cannot hide a key use or a literal.
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
use syn::{
    Attribute, Expr, ExprCall, ExprLit, ExprMethodCall, ItemMod, Lit, LitStr, Path as SynPath,
    visit::Visit,
};

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    variant: String,
    key: String,
    en: String,
    vi: String,
}

pub fn check(root: &Path) -> Result<(), String> {
    let catalog_path = root.join("apps/vibecut/src/catalog.rs");
    let catalog = fs::read_to_string(&catalog_path)
        .map_err(|e| format!("{}: {e}", catalog_path.display()))?;
    let entries = parse(&catalog)?;
    let mut errors = Vec::new();
    let mut keys = BTreeSet::new();
    let mut variants = BTreeSet::new();
    for entry in &entries {
        if !keys.insert(entry.key.clone()) {
            errors.push(format!("duplicate catalog key `{}`", entry.key));
        }
        if !variants.insert(entry.variant.clone()) {
            errors.push(format!("duplicate catalog variant `{}`", entry.variant));
        }
        if entry.key.trim().is_empty() {
            errors.push(format!("`{}` has an empty key", entry.variant));
        }
        if entry.en.trim().is_empty() {
            errors.push(format!("`{}` has an empty English row", entry.key));
        }
        if entry.vi.trim().is_empty() {
            errors.push(format!("`{}` has an empty Vietnamese row", entry.key));
        }
    }

    let sources = rust_sources(&root.join("apps/vibecut/src"))?;
    let mut used = BTreeSet::new();
    for (path, text) in &sources {
        let parsed = syn::parse_file(text)
            .map_err(|error| format!("{}: invalid Rust: {error}", path.display()))?;
        let mut audit = Audit {
            path,
            scan_macros: path.file_name().is_none_or(|name| name != "catalog.rs"),
            used: BTreeSet::new(),
            errors: Vec::new(),
        };
        audit.visit_file(&parsed);
        used.extend(audit.used);
        errors.extend(audit.errors);
    }
    for entry in &entries {
        if !used.contains(&entry.variant) {
            errors.push(format!(
                "unused catalog key `{}` (Text::{} is not referenced by production frontend code)",
                entry.key, entry.variant
            ));
        }
    }

    if errors.is_empty() {
        eprintln!(
            "i18n: ok ({} keys, English and Vietnamese complete, no unused keys, no direct UI literals)",
            entries.len()
        );
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

fn rust_sources(dir: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_owned()];
    while let Some(dir) = pending.pop() {
        let entries = fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let text =
                    fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                out.push((path, text));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

const VISIBLE_TYPES: &[&str] = &[
    "RichText",
    "Button",
    "Label",
    "SelectableLabel",
    "Checkbox",
    "RadioButton",
    "CollapsingHeader",
    "Window",
    "Hyperlink",
    "Link",
    "ComboBox",
];
const FIRST_ARG_METHODS: &[&str] = &[
    "label",
    "heading",
    "button",
    "small_button",
    "menu_button",
    "hyperlink",
    "on_hover_text",
    "on_disabled_hover_text",
    "hint_text",
    "add_filter",
    "set_file_name",
    "from_label",
];
const ALL_ARG_METHODS: &[&str] = &[
    "selectable_label",
    "checkbox",
    "radio_value",
    "toggle_value",
    "text",
];
const FIRST_ARG_FUNCTIONS: &[&str] = &["run_native", "from_label"];

struct Audit<'a> {
    path: &'a Path,
    scan_macros: bool,
    used: BTreeSet<String>,
    errors: Vec<String>,
}

impl Audit<'_> {
    fn literal(&mut self, expression: &Expr, callee: &str) {
        let Expr::Lit(ExprLit {
            lit: Lit::Str(value),
            ..
        }) = expression
        else {
            return;
        };
        self.report(value, callee);
    }
    fn first_literal(
        &mut self,
        args: &syn::punctuated::Punctuated<Expr, syn::Token![,]>,
        callee: &str,
    ) {
        if let Some(expression) = args.first() {
            self.literal(expression, callee);
        }
    }
    fn all_literals(
        &mut self,
        args: &syn::punctuated::Punctuated<Expr, syn::Token![,]>,
        callee: &str,
    ) {
        for expression in args {
            self.literal(expression, callee);
        }
    }
    fn report(&mut self, value: &LitStr, callee: &str) {
        let line = value.span().start().line;
        self.errors.push(format!(
            "{}:{line}: string literal in `{callee}`; route it through the catalog",
            self.path.display()
        ));
    }
}
impl<'ast> Visit<'ast> for Audit<'_> {
    fn visit_path(&mut self, path: &'ast SynPath) {
        let segments: Vec<String> = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect();
        if let Some(owner) = segments.get(segments.len().saturating_sub(2))
            && owner == "Text"
            && let Some(variant) = segments.last()
        {
            self.used.insert(variant.clone());
        }
        syn::visit::visit_path(self, path);
    }

    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        if is_cfg_test(&item.attrs) {
            return;
        }
        syn::visit::visit_item_mod(self, item);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if self.scan_macros {
            scan_macro_tokens(mac.tokens.clone(), &mut self.used);
        }
        syn::visit::visit_macro(self, mac);
    }

    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if let Expr::Path(path) = call.func.as_ref() {
            let segments: Vec<String> = path
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect();
            let callee = segments.join("::");
            let last = segments.last().map_or("", String::as_str);
            if FIRST_ARG_FUNCTIONS.contains(&last) {
                self.first_literal(&call.args, &callee);
            } else if segments
                .iter()
                .any(|segment| VISIBLE_TYPES.contains(&segment.as_str()))
            {
                self.all_literals(&call.args, &callee);
            }
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        let callee = call.method.to_string();
        if FIRST_ARG_METHODS.contains(&callee.as_str()) {
            self.first_literal(&call.args, &callee);
        } else if ALL_ARG_METHODS.contains(&callee.as_str()) {
            self.all_literals(&call.args, &callee);
        }
        syn::visit::visit_expr_method_call(self, call);
    }
}

fn scan_macro_tokens(tokens: proc_macro2::TokenStream, used: &mut BTreeSet<String>) {
    let tokens: Vec<proc_macro2::TokenTree> = tokens.into_iter().collect();
    for (index, token) in tokens.iter().enumerate() {
        if let proc_macro2::TokenTree::Group(group) = token {
            scan_macro_tokens(group.stream(), used);
        }
        let proc_macro2::TokenTree::Ident(ident) = token else {
            continue;
        };
        if ident != "Text" {
            continue;
        }
        let colon = |offset: usize| {
            matches!(
                tokens.get(index + offset),
                Some(proc_macro2::TokenTree::Punct(punct)) if punct.as_char() == ':'
            )
        };
        if colon(1)
            && colon(2)
            && let Some(proc_macro2::TokenTree::Ident(variant)) = tokens.get(index + 3)
        {
            used.insert(variant.to_string());
        }
    }
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

fn parse(source: &str) -> Result<Vec<Entry>, String> {
    let start = source
        .rfind("catalog!")
        .ok_or("catalog.rs has no catalog! invocation")?;
    let source = source
        .get(start..)
        .ok_or_else(|| format!("catalog invocation is not UTF-8 near byte {start}"))?;
    let mut entries = Vec::new();
    let mut search = 0;
    while let Some(relative) = source[search..].find("en:") {
        let en_colon = search + relative;
        let open = source[..en_colon]
            .rfind('{')
            .ok_or_else(|| format!("catalog `en:` without an entry near byte {en_colon}"))?;
        let header = source
            .get(open + 1..en_colon)
            .ok_or_else(|| format!("catalog entry is not UTF-8 near byte {en_colon}"))?;
        let key = field(header, "key:")?;
        let (en, after_en) = quoted(source, en_colon + 3)?;
        let close = source
            .get(after_en..)
            .and_then(|rest| rest.find('}').map(|index| after_en + index))
            .ok_or_else(|| format!("catalog `{key}` entry has no closing brace"))?;
        let vi_colon = source
            .get(after_en..close)
            .and_then(|rest| rest.find("vi:").map(|index| after_en + index))
            .ok_or_else(|| format!("catalog `{key}` has no Vietnamese row"))?;
        let (vi, after_vi) = quoted(source, vi_colon + 3)?;
        let before = source
            .get(..open)
            .ok_or_else(|| format!("catalog entry is not UTF-8 near byte {open}"))?
            .trim_end();
        let start = before
            .rfind(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .map_or(0, |index| index + 1);
        let variant = before
            .get(start..)
            .filter(|variant| {
                variant
                    .chars()
                    .next()
                    .is_some_and(|first| first.is_ascii_uppercase())
            })
            .ok_or_else(|| format!("catalog entry near byte {open} has no variant name"))?;
        entries.push(Entry {
            variant: variant.to_owned(),
            key,
            en,
            vi,
        });
        search = after_vi;
    }
    if entries.is_empty() {
        return Err("catalog.rs contains no catalog rows".into());
    }
    Ok(entries)
}

fn field(header: &str, name: &str) -> Result<String, String> {
    let at = header
        .find(name)
        .ok_or_else(|| format!("catalog entry has no `{name}`"))?;
    quoted(header, at + name.len()).map(|(value, _)| value)
}

fn quoted(source: &str, at: usize) -> Result<(String, usize), String> {
    let mut rest = source
        .get(at..)
        .ok_or_else(|| format!("catalog value is not UTF-8 near byte {at}"))?;
    let skipped = rest.len() - rest.trim_start().len();
    rest = rest.trim_start();
    let mut chars = rest.char_indices();
    if chars.next().map(|(_, character)| character) != Some('"') {
        return Err(format!(
            "catalog value near byte {} is not a string",
            at + skipped
        ));
    }
    let mut value = String::new();
    while let Some((index, character)) = chars.next() {
        match character {
            '\\' => {
                let (_, escaped) = chars
                    .next()
                    .ok_or_else(|| format!("catalog string near byte {at} ends in `\\`"))?;
                value.push(match escaped {
                    'n' => '\n',
                    't' => '\t',
                    '"' => '"',
                    '\\' => '\\',
                    other => other,
                });
            }
            '"' => return Ok((value, at + skipped + index + 1)),
            character => value.push(character),
        }
    }
    Err(format!("catalog string near byte {at} is unterminated"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "vibecut-i18n-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(path.join("apps/vibecut/src")).unwrap();
        path
    }

    #[test]
    fn parser_accepts_wrapped_rows_and_rejects_missing_vi() {
        let rows = r#"
            catalog! {
                First { key: "first", en: "One", vi: "Một" },
                Second {
                    key: "second",
                    en: "Two",
                    vi: "Hai",
                },
            }
        "#;
        let parsed = parse(rows).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[1].variant, "Second");
        assert_eq!(parsed[1].vi, "Hai");
        assert!(parse("catalog! { Broken { key: \"broken\", en: \"No\" } }").is_err());
    }

    #[test]
    fn audit_uses_exact_keys_and_catches_multiline_and_raw_literals() {
        let root = tempdir();
        let catalog = r#"catalog! {
            Save { key: "save", en: "Save", vi: "Lưu" },
            SaveAs { key: "save_as", en: "Save as", vi: "Lưu thành" },
            Empty { key: "empty", en: "", vi: "Trống" },
            Unused { key: "unused", en: "Unused", vi: "Không dùng" },
        }"#;
        fs::write(root.join("apps/vibecut/src/catalog.rs"), catalog).unwrap();
        fs::write(
            root.join("apps/vibecut/src/frontend.rs"),
            r##"
fn ui() {
    Text::SaveAs;
    // Text::Save must not count: comments are not production uses.
    ui.label(
        "Hard-coded English",
    );
    ui.label ( r#"Hard-coded raw"# );
    egui::RichText::new(
        "Hard-coded rich",
    );
    builder.add_filter("Hard-coded filter", &["vibecut", "json"]);
    builder.set_file_name("project.vibecut");
    egui::Button::image_and_text(image, "Hard-coded button");
}
"##,
        )
        .unwrap();
        let error = check(&root).unwrap_err();
        assert!(error.contains("empty English row"), "{error}");
        assert!(error.contains("unused catalog key `save`"), "{error}");
        assert!(error.contains("unused catalog key `unused`"), "{error}");
        for callee in [
            "label",
            "RichText::new",
            "add_filter",
            "set_file_name",
            "image_and_text",
        ] {
            assert!(error.contains(callee), "{callee}: {error}");
        }
        fs::remove_dir_all(root).unwrap();
    }
}
