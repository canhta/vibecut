//! Generated-tree tests for the structure gate: each rule passes and fails.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use super::{BASELINE, CONFIG, check, imports::control_order};
use std::{
    fs,
    path::{Path, PathBuf},
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
        fs::create_dir_all(dir.join("crates/engine/tests/it/control")).unwrap();
        fs::write(
            dir.join("crates/engine/tests/it/control/catalog.rs"),
            "use crate::support::control::Allow;\n",
        )
        .unwrap();
        git(&dir, &["init", "--quiet"]);
        Self(dir)
    }

    fn write(&self, path: &str, text: &str) {
        let file = self.0.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
        git(&self.0, &["-c", "core.autocrlf=false", "add", "--", path]);
    }

    fn remove(&self, path: &str) {
        fs::remove_file(self.0.join(path)).unwrap();
        git(&self.0, &["rm", "--quiet", "--cached", "--", path]);
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

#[test]
fn doc_line_edge_cases() {
    let tree = Tree::new();
    tree.write("crates/foo/src/lib.rs", "//! Foo.\nmod thing;\nmod wrap;\n");
    tree.write("crates/foo/src/thing.rs", "\u{feff}//! Thing.\n");
    tree.write("crates/foo/src/wrap.rs", "//! Wrap.\nmod inner;\n");
    tree.write("crates/foo/src/wrap/inner.rs", "//! Inner.\n");
    check(&tree.0).unwrap();
    // attribute first, mod-only files, an empty file and the module file beside its folder
    tree.write(
        "crates/foo/src/thing.rs",
        "#![forbid(unsafe_code)]\n//! Thing.\n",
    );
    tree.write("crates/foo/src/wrap.rs", "mod inner;\n");
    tree.write("crates/foo/src/wrap/inner.rs", "");
    let error = check(&tree.0).unwrap_err();
    for expected in [
        "crates/foo/src/thing.rs: [doc]",
        "crates/foo/src/wrap.rs: [doc]",
        "crates/foo/src/wrap/inner.rs: [doc]",
    ] {
        assert!(error.contains(expected), "{expected}: {error}");
    }
    assert!(!error.contains("empty Rust source"), "{error}");
}

#[test]
fn mod_rs_fails_in_src_and_tests() {
    let tree = Tree::new();
    tree.write("crates/foo/src/lib.rs", "//! Foo.\n");
    tree.write("crates/foo/src/thing/mod.rs", "//! Thing.\n");
    let error = check(&tree.0).unwrap_err();
    for expected in [
        "crates/foo/src/thing/mod.rs",
        "[interface]",
        "structure rule 3",
        "use `foo.rs` beside `foo/`",
    ] {
        assert!(error.contains(expected), "{expected}: {error}");
    }
    tree.remove("crates/foo/src/thing/mod.rs");
    tree.write("crates/foo/src/thing.rs", "//! Thing.\n");
    tree.write("crates/foo/src/thing/child.rs", "//! Child.\n");
    check(&tree.0).unwrap();

    tree.write("crates/foo/tests/it/mod.rs", "mod case;\n");
    let error = check(&tree.0).unwrap_err();
    assert!(
        error.contains("crates/foo/tests/it/mod.rs: [interface]"),
        "{error}"
    );
    assert!(!error.contains("[doc]"), "{error}");
    tree.remove("crates/foo/tests/it/mod.rs");
    tree.write("crates/foo/tests/it/main.rs", "mod case;\n");
    check(&tree.0).unwrap();
}

#[test]
fn banned_names_cover_test_module_trees() {
    let tree = Tree::new();
    tree.write("crates/foo/src/lib.rs", "//! Foo.\n");
    tree.write("crates/foo/tests/it/support.rs", "//! Support.\n");
    check(&tree.0).unwrap();
    tree.write("crates/foo/tests/it/helpers.rs", "//! Helpers.\n");
    let error = check(&tree.0).unwrap_err();
    for expected in [
        "crates/foo/tests/it/helpers.rs",
        "[name]",
        "`helpers` is banned",
    ] {
        assert!(error.contains(expected), "{expected}: {error}");
    }
    tree.remove("crates/foo/tests/it/helpers.rs");
    tree.write(
        "crates/foo/tests/it/support.rs",
        "//! Support.\nmod misc {}\n",
    );
    let error = check(&tree.0).unwrap_err();
    assert!(
        error.contains("crates/foo/tests/it/support.rs: [name]")
            && error.contains("`misc` is banned"),
        "{error}"
    );
    tree.write("crates/foo/tests/it/support.rs", "//! Support.\n");
    tree.write("crates/foo/tests/it/common/case.rs", "//! Case.\n");
    let error = check(&tree.0).unwrap_err();
    assert!(
        error.contains("crates/foo/tests/it/common: [name]")
            && error.contains("`common` is banned"),
        "{error}"
    );
    tree.remove("crates/foo/tests/it/common/case.rs");
    check(&tree.0).unwrap();
}

const EDITOR_FOLDER: &str = r#"[[crate]]
name = "foo"
path = "crates/foo"

[[crate.layer]]
name = "base"
groups = ["base"]

[[crate.module]]
name = "editor"
group = "base"

[[crate.folder]]
path = "src/editor"

[[crate.folder.layer]]
name = "features"
groups = ["assets", "player"]

[[crate.folder.layer]]
name = "frame"
groups = ["frame"]

[[crate.folder.module]]
name = "frame"
group = "frame"

[[crate.folder.module]]
name = "assets"
group = "assets"

[[crate.folder.module]]
name = "player"
group = "player"
"#;

#[test]
fn folder_tables_enforce_layers_and_groups_below_a_folder_interface() {
    // Layers list lowest first: the frame sits above the areas and may call them.
    let tree = Tree::new();
    tree.config(EDITOR_FOLDER);
    tree.write("crates/foo/src/lib.rs", "//! Foo.\npub mod editor;\n");
    tree.write(
        "crates/foo/src/editor.rs",
        "//! Editor.\npub mod frame;\npub mod assets;\npub mod player;\n",
    );
    tree.write(
        "crates/foo/src/editor/frame.rs",
        "//! Frame.\nuse crate::editor::assets::Assets;\nuse crate::editor::player::Player;\npub struct Frame;\n",
    );
    tree.write(
        "crates/foo/src/editor/assets.rs",
        "//! Assets.\nuse crate::editor::frame::Frame;\npub struct Assets;\n",
    );
    tree.write(
        "crates/foo/src/editor/player.rs",
        "//! Player.\npub struct Player;\n",
    );
    let error = check(&tree.0).unwrap_err();
    for expected in [
        "crates/foo/src/editor/assets.rs",
        "[layer]",
        "structure rule 4",
        "higher layer",
        "inside folder `src/editor`",
    ] {
        assert!(error.contains(expected), "{expected}: {error}");
    }
    assert!(!error.contains("editor/frame.rs"), "{error}");
    tree.write(
        "crates/foo/src/editor/assets.rs",
        "//! Assets.\nuse crate::editor::player::Player;\npub struct Assets;\n",
    );
    let error = check(&tree.0).unwrap_err();
    assert!(
        error.contains("crosses feature groups") && error.contains("inside folder `src/editor`"),
        "{error}"
    );
    tree.write(
        "crates/foo/src/editor/assets.rs",
        "//! Assets.\npub struct Assets;\n",
    );
    check(&tree.0).unwrap();
}

#[test]
fn folder_tables_reach_nested_importers_and_relative_paths() {
    let tree = Tree::new();
    tree.config(EDITOR_FOLDER);
    tree.write("crates/foo/src/lib.rs", "//! Foo.\npub mod editor;\n");
    tree.write(
        "crates/foo/src/editor.rs",
        "//! Editor.\npub mod frame;\npub mod assets;\npub mod player;\n",
    );
    tree.write("crates/foo/src/editor/frame.rs", "//! Frame.\n");
    tree.write(
        "crates/foo/src/editor/assets.rs",
        "//! Assets.\npub struct Assets;\n",
    );
    tree.write(
        "crates/foo/src/editor/player.rs",
        "//! Player.\nmod view;\n",
    );
    tree.write(
        "crates/foo/src/editor/player/view.rs",
        "//! View.\nuse crate::editor::assets::Assets;\n",
    );
    let error = check(&tree.0).unwrap_err();
    assert!(
        error.contains("crates/foo/src/editor/player/view.rs: [layer]")
            && error.contains("crosses feature groups"),
        "{error}"
    );
    tree.write(
        "crates/foo/src/editor/player/view.rs",
        "//! View.\nuse super::super::assets::Assets;\n",
    );
    let error = check(&tree.0).unwrap_err();
    assert!(error.contains("crosses feature groups"), "{error}");
    tree.write(
        "crates/foo/src/editor/player/view.rs",
        "//! View.\nuse crate::editor::frame::Frame;\n",
    );
    let error = check(&tree.0).unwrap_err();
    assert!(error.contains("higher layer"), "{error}");
    tree.write("crates/foo/src/editor/player/view.rs", "//! View.\n");
    check(&tree.0).unwrap();
}

#[test]
fn folder_tables_reject_unplaced_and_stale_folder_modules() {
    let tree = Tree::new();
    tree.config(EDITOR_FOLDER);
    tree.write("crates/foo/src/lib.rs", "//! Foo.\npub mod editor;\n");
    tree.write(
        "crates/foo/src/editor.rs",
        "//! Editor.\npub mod frame;\npub mod assets;\npub mod player;\npub mod missing;\n",
    );
    tree.write("crates/foo/src/editor/frame.rs", "//! Frame.\n");
    tree.write("crates/foo/src/editor/assets.rs", "//! Assets.\n");
    tree.write("crates/foo/src/editor/player.rs", "//! Player.\n");
    tree.write("crates/foo/src/editor/missing.rs", "//! Missing.\n");
    let error = check(&tree.0).unwrap_err();
    assert!(
        error.contains("folder `src/editor` module `missing` has no layer and group"),
        "{error}"
    );
    tree.write(
        "crates/foo/src/editor.rs",
        "//! Editor.\npub mod frame;\npub mod assets;\npub mod player;\n",
    );
    tree.remove("crates/foo/src/editor/missing.rs");
    check(&tree.0).unwrap();
    tree.remove("crates/foo/src/editor/player.rs");
    let error = check(&tree.0).unwrap_err();
    assert!(
        error.contains("folder `src/editor` places module `player`, which no longer exists"),
        "{error}"
    );
}

#[test]
fn forbidden_frame_type_fails_in_listed_layers_only() {
    let tree = Tree::new();
    tree.config(&format!(
        "{EDITOR_FOLDER}\n[[crate.folder.forbid]]\nname = \"Editor\"\nlayers = [\"features\"]\n"
    ));
    tree.write("crates/foo/src/lib.rs", "//! Foo.\npub mod editor;\n");
    tree.write(
        "crates/foo/src/editor.rs",
        "//! Editor.\npub mod frame;\npub mod assets;\npub mod player;\n",
    );
    tree.write(
        "crates/foo/src/editor/frame.rs",
        "//! Frame.\npub struct Editor;\nfn f(_: &Editor) {}\n",
    );
    tree.write(
        "crates/foo/src/editor/player.rs",
        "//! Player.\n// The Editor is only a word here.\npub fn f() -> &'static str { \"Editor\" }\n#[cfg(test)]\nfn t(_: &Editor) {}\n",
    );
    tree.write(
        "crates/foo/src/editor/assets.rs",
        "//! Assets.\npub fn f(_: &mut crate::editor::Editor) {}\n",
    );
    let error = check(&tree.0).unwrap_err();
    for expected in [
        "crates/foo/src/editor/assets.rs",
        "[frame-type]",
        "structure rule 5",
        "names `Editor`",
    ] {
        assert!(error.contains(expected), "{expected}: {error}");
    }
    assert!(!error.contains("editor/frame.rs"), "{error}");
    assert!(!error.contains("editor/player.rs"), "{error}");
    tree.baseline(&[(
        "frame-type",
        "crates/foo/src/editor/assets.rs",
        "names `Editor`, the frame record; take a port instead",
    )]);
    check(&tree.0).unwrap();
    tree.write(
        "crates/foo/src/editor/assets.rs",
        "//! Assets.\npub fn f() {}\n",
    );
    let error = check(&tree.0).unwrap_err();
    assert!(error.contains("baseline entry no longer occurs"), "{error}");
}

#[test]
fn banned_names_match_whole_names_in_any_case_and_inline_modules() {
    let tree = Tree::new();
    tree.write("crates/foo/src/lib.rs", "//! Foo.\n");
    tree.write("crates/foo/src/utils_x.rs", "//! Fine.\n");
    tree.write("crates/foo/src/my_core/part.rs", "//! Fine.\n");
    tree.write(
        "crates/foo/src/ok.rs",
        "//! Ok.\n#[cfg(test)]\nmod helpers {}\n",
    );
    check(&tree.0).unwrap();
    tree.write("crates/foo/src/Utils.rs", "//! Mixed case.\n");
    tree.write("crates/foo/src/types/part.rs", "//! Part.\n");
    tree.write(
        "crates/foo/src/ok.rs",
        "//! Ok.\nmod misc {\n    pub fn f() {}\n}\n",
    );
    let error = check(&tree.0).unwrap_err();
    for expected in [
        "crates/foo/src/Utils.rs: [name]",
        "crates/foo/src/types: [name]",
        "crates/foo/src/ok.rs: [name]",
        "`misc` is banned",
    ] {
        assert!(error.contains(expected), "{expected}: {error}");
    }
    assert!(!error.contains("utils_x"), "{error}");
    assert!(!error.contains("my_core"), "{error}");
}

#[test]
fn module_file_beside_its_folder_counts_once() {
    let tree = Tree::new();
    tree.write("crates/foo/src/lib.rs", "//! Foo.\n");
    for index in 0..11 {
        tree.write(
            &format!("crates/foo/src/wide/part{index}.rs"),
            "//! Part.\n",
        );
    }
    tree.write("crates/foo/src/wide/group.rs", "//! Group.\n");
    tree.write("crates/foo/src/wide/group/leaf.rs", "//! Leaf.\n");
    tree.write("crates/foo/src/wide/tests/case.rs", "//! Case.\n");
    tree.write("crates/foo/src/wide/part0_tests.rs", "//! Cases.\n");
    tree.write("crates/foo/src/wide/README.md", "# Wide\n");
    check(&tree.0).unwrap();
    tree.write("crates/foo/src/wide/extra.rs", "//! Extra.\n");
    assert!(check(&tree.0).unwrap_err().contains("13 direct children"));
}

#[test]
fn import_scan_ignores_non_code_and_test_only_code() {
    let tree = Tree::new();
    tree.config(LAYERS);
    tree.write(
        "crates/foo/src/lib.rs",
        "//! Foo.\npub mod low;\npub mod one;\npub mod two;\npub mod high;\npub use high::High;\n",
    );
    tree.write("crates/foo/src/low.rs", "//! Low.\npub struct Low;\n");
    tree.write("crates/foo/src/two.rs", "//! Two.\npub struct Two;\n");
    tree.write("crates/foo/src/high.rs", "//! High.\npub struct High;\n");
    tree.write(
        "crates/foo/src/one.rs",
        r#"//! One.
//! See crate::two::Two and `use crate::high::High`.
/// Docs mention crate::two::Two too.
pub const TEXT: &str = "crate::two::Two";
macro_rules! shout { () => { crate::two::Two } }
#[cfg(test)]
use crate::two::Two;
#[cfg(all(test, unix))]
mod unix_tests {
use crate::two::Two;
}
#[cfg(test)]
fn helper() -> crate::two::Two { todo!() }
"#,
    );
    check(&tree.0).unwrap();
    for (name, body, expected) in [
        (
            "grouped",
            "use crate::{low::Low, two::Two};\n",
            "crosses feature groups",
        ),
        (
            "not_test",
            "#[cfg(not(test))]\nuse crate::two::Two;\n",
            "crosses feature groups",
        ),
        (
            "feature_named_test",
            "#[cfg(feature = \"test-utils\")]\nuse crate::two::Two;\n",
            "crosses feature groups",
        ),
        ("reexport", "use crate::High;\n", "crosses feature groups"),
        (
            "expression",
            "fn f() { let _ = crate::two::Two; }\n",
            "crosses feature groups",
        ),
        (
            "nested_super",
            "mod inner { use super::super::two::Two; }\n",
            "crosses feature groups",
        ),
    ] {
        tree.write("crates/foo/src/one.rs", &format!("//! One.\n{body}"));
        let error = check(&tree.0).unwrap_err();
        assert!(error.contains(expected), "{name}: {error}");
        assert!(error.contains("crates/foo/src/one.rs"), "{name}: {error}");
    }
    tree.write(
        "crates/foo/src/low.rs",
        "//! Low.\npub use crate::high::High;\n",
    );
    tree.write("crates/foo/src/one.rs", "//! One.\n");
    let error = check(&tree.0).unwrap_err();
    assert!(error.contains("crates/foo/src/low.rs"), "pub use: {error}");
    assert!(error.contains("higher layer"), "pub use: {error}");
}

#[test]
fn folder_children_may_use_each_other_but_not_reach_into_other_folders() {
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
        "//! Foo.\nmod bar;\nmod baz;\npub use bar::Thing;\n",
    );
    tree.write("crates/foo/src/foo/bar.rs", "//! Bar.\npub struct Thing;\n");
    tree.write(
        "crates/foo/src/foo/baz.rs",
        "//! Baz.\nuse super::bar::Thing;\nuse crate::foo::bar::Thing as Other;\n",
    );
    tree.write(
        "crates/foo/src/user.rs",
        "//! User.\nuse crate::foo::Thing;\n",
    );
    check(&tree.0).unwrap();
    tree.write(
        "crates/foo/src/user.rs",
        "//! User.\nuse crate::foo::bar;\n",
    );
    assert!(check(&tree.0).unwrap_err().contains("[interface]"));
}

#[test]
fn baseline_must_be_sorted_unique_and_well_formed() {
    let tree = Tree::new();
    tree.write("crates/foo/src/lib.rs", "//! Foo.\n");
    let a = "doc\tcrates/foo/src/a.rs\tfirst line is not a //! responsibility line";
    let b = "doc\tcrates/foo/src/b.rs\tfirst line is not a //! responsibility line";
    for (text, expected) in [
        (format!("{b}\n{a}\n"), "must be sorted"),
        (format!("{a}\n{a}\n"), "without duplicates"),
        (
            "doc\tcrates/foo/src/a.rs\n".to_owned(),
            "expected `rule<TAB>path<TAB>detail`",
        ),
        ("nope\tcrates/foo/src/a.rs\tx\n".to_owned(), "unknown rule"),
    ] {
        fs::write(tree.0.join(BASELINE), text).unwrap();
        let error = check(&tree.0).unwrap_err();
        assert!(error.contains(expected), "{expected}: {error}");
    }
}

#[test]
fn root_module_aliases_resolve_and_stale_table_entries_fail() {
    let tree = Tree::new();
    tree.config(LAYERS);
    tree.write(
        "crates/foo/src/lib.rs",
        "//! Foo.\nmod low;\nmod one;\nmod two;\nmod high;\nuse high as top;\n",
    );
    tree.write("crates/foo/src/low.rs", "//! Low.\nuse crate::top::High;\n");
    tree.write("crates/foo/src/one.rs", "//! One.\n");
    tree.write("crates/foo/src/two.rs", "//! Two.\n");
    tree.write("crates/foo/src/high.rs", "//! High.\npub struct High;\n");
    let error = check(&tree.0).unwrap_err();
    assert!(
        error.contains("crates/foo/src/low.rs: [layer]") && error.contains("higher layer"),
        "{error}"
    );
    tree.write("crates/foo/src/low.rs", "//! Low.\n");
    check(&tree.0).unwrap();
    fs::remove_file(tree.0.join("crates/foo/src/two.rs")).unwrap();
    git(
        &tree.0,
        &["rm", "--quiet", "--cached", "--", "crates/foo/src/two.rs"],
    );
    let error = check(&tree.0).unwrap_err();
    assert!(
        error.contains("`two`") && error.contains("no longer exists"),
        "{error}"
    );
}

#[test]
fn control_support_imports_are_one_sorted_name_per_line() {
    let sorted = "\
use crate::support::control::Allow;
use crate::support::control::finish;
use crate::support::control::write_wav;
";
    control_order(sorted).unwrap();
    let disordered = "\
use crate::support::control::Allow;
use crate::support::control::write_wav;
use crate::support::control::finish;
";
    let error = control_order(disordered).unwrap_err();
    assert!(error.contains("`finish`"), "{error}");
    assert!(error.contains("sorts before `write_wav`"), "{error}");
    let grouped = "\
use crate::support::control::{Allow, finish};
";
    let error = control_order(grouped).unwrap_err();
    assert!(error.contains("one name per line"), "{error}");
    let missing = "fn main() {}\n";
    let error = control_order(missing).unwrap_err();
    assert!(
        error.contains("no `use crate::support::control::"),
        "{error}"
    );
}
