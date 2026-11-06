//! Generated-tree tests for the documentation gate: each rule passes and fails.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use super::{BASELINE, check};
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
            "vibecut-docs-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(dir.join("xtask")).unwrap();
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

    fn remove(&self, path: &str) {
        fs::remove_file(self.0.join(path)).unwrap();
        git(&self.0, &["rm", "--quiet", "--cached", "--", path]);
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

#[test]
fn docs_index_lists_every_tracked_page() {
    let tree = Tree::new();
    tree.write("docs/README.md", "# Docs\n");
    tree.write("docs/engineering/one.md", "# One\n");
    let error = check(&tree.0).unwrap_err();
    for expected in [
        "docs/engineering/one.md",
        "[index]",
        "not listed in docs/README.md",
    ] {
        assert!(error.contains(expected), "{expected}: {error}");
    }
    tree.write(
        "docs/README.md",
        "# Docs\n\nengineering/one.md is mentioned without a link\n",
    );
    assert!(check(&tree.0).is_err());
    tree.write("docs/README.md", "# Docs\n\n[One](engineering/one.md)\n");
    check(&tree.0).unwrap();
}

#[test]
fn module_maps_cover_folder_modules_and_reject_stale_rows() {
    let tree = Tree::new();
    tree.write("docs/README.md", "# Docs\n");
    tree.write("crates/foo/src/lib.rs", "//! Foo.\n");
    tree.write("crates/foo/src/editor.rs", "//! Editor.\n");
    tree.write("crates/foo/README.md", "# foo\n");
    let error = check(&tree.0).unwrap_err();
    assert!(
        error.contains("crates/foo/README.md: [map] docs check: README has no module map table"),
        "{error}"
    );
    tree.write(
        "crates/foo/README.md",
        "# foo\n\n| Folder | Responsibility |\n|---|---|\n| `editor/` | Editor. |\n",
    );
    check(&tree.0).unwrap();
    tree.write("crates/foo/src/editor/assets.rs", "//! Assets.\n");
    tree.write("crates/foo/src/editor/assets/view.rs", "//! View.\n");
    let error = check(&tree.0).unwrap_err();
    assert!(
        error.contains("module map has no row for folder `editor/assets`"),
        "{error}"
    );
    tree.write(
        "crates/foo/README.md",
        "# foo\n\n| Folder | Responsibility |\n|---|---|\n| `editor/` | Editor. |\n| `editor/assets/` | Assets. |\n",
    );
    check(&tree.0).unwrap();
    tree.write(
        "crates/foo/README.md",
        "# foo\n\n| Folder | Responsibility |\n|---|---|\n| `editor/` | Editor. |\n| `editor/assets/` | Assets. |\n| `gone/` | Gone. |\n",
    );
    let error = check(&tree.0).unwrap_err();
    assert!(
        error.contains("module map row `gone` names no module under src"),
        "{error}"
    );
    tree.remove("crates/foo/src/editor/assets/view.rs");
    tree.write(
        "crates/foo/README.md",
        "# foo\n\n| Folder | Responsibility |\n|---|---|\n| `editor/` | Editor. |\n",
    );
    check(&tree.0).unwrap();
}

#[test]
fn crate_and_app_readmes_without_a_map_fail() {
    let tree = Tree::new();
    tree.write("docs/README.md", "# Docs\n");
    tree.write("crates/time/src/lib.rs", "//! Time.\n");
    tree.write("crates/time/src/speed.rs", "//! Speed.\n");
    tree.write("crates/time/README.md", "# time\n");
    tree.write("apps/vibecut/src/lib.rs", "//! App.\n");
    tree.write("apps/vibecut/README.md", "# vibecut\n");
    let error = check(&tree.0).unwrap_err();
    for expected in [
        "crates/time/README.md",
        "apps/vibecut/README.md",
        "README has no module map table",
    ] {
        assert!(error.contains(expected), "{expected}: {error}");
    }
    tree.write(
        "crates/time/README.md",
        "# time\n\n| Module | Responsibility |\n|---|---|\n| `speed` | Speed. |\n",
    );
    tree.write(
        "apps/vibecut/README.md",
        "# vibecut\n\n| Module | Responsibility |\n|---|---|\n",
    );
    check(&tree.0).unwrap();
    tree.write("apps/vibecut/src/editor.rs", "//! Editor.\n");
    tree.write("apps/vibecut/src/editor/frame.rs", "//! Frame.\n");
    let error = check(&tree.0).unwrap_err();
    assert!(
        error.contains("module map has no row for folder `editor`"),
        "{error}"
    );
}

#[test]
fn tracker_ids_fail_and_the_baseline_only_shrinks() {
    let tree = Tree::new();
    tree.write("docs/README.md", "# Docs\n\n[Notes](notes.md)\n");
    tree.write(
        "docs/notes.md",
        "# Notes\n\nSee p0-core/12 and ticket 34.\n",
    );
    let error = check(&tree.0).unwrap_err();
    for expected in [
        "docs/notes.md",
        "[text]",
        "tracker id `p0-core/12`",
        "tracker id `ticket 34`",
    ] {
        assert!(error.contains(expected), "{expected}: {error}");
    }
    tree.baseline(&[
        ("text", "docs/notes.md", "tracker id `p0-core/12`"),
        ("text", "docs/notes.md", "tracker id `ticket 34`"),
    ]);
    check(&tree.0).unwrap();
    tree.write("docs/notes.md", "# Notes\n\nClean.\n");
    let error = check(&tree.0).unwrap_err();
    assert!(error.contains("no longer occurs"), "{error}");
    assert!(error.contains("remove it"), "{error}");
}

#[test]
fn forbidden_names_fail_but_attribution_added_fields_are_exempt() {
    let tree = Tree::new();
    tree.write("docs/README.md", "# Docs\n\n[Notes](notes.md)\n");
    fs::create_dir_all(tree.0.join(".scratch")).unwrap();
    fs::write(
        tree.0.join(".scratch/forbidden-names.txt"),
        "sentinelhost\n",
    )
    .unwrap();
    tree.write("docs/notes.md", "# Notes\n\nSentinelHost appears here.\n");
    let error = check(&tree.0).unwrap_err();
    assert!(
        error.contains("docs/notes.md: [text] docs check: contains a forbidden host or agent name"),
        "{error}"
    );
    tree.write("docs/notes.md", "# Notes\n\nadded: sentinelhost\n");
    check(&tree.0).unwrap();
}

#[test]
fn tracker_scan_has_word_boundaries_and_skips_attribution_lines() {
    let ids = super::text::tracker_ids("see p0-core/94, P1-Parity/119 and ticket 11");
    assert_eq!(
        ids,
        ["p0-core/94", "p1-parity/119", "ticket 11"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
    assert!(super::text::tracker_ids("mp4/core/1 and xp1-core/2").is_empty());
    assert!(super::text::tracker_ids("ticketed 3 and ticket 3x").is_empty());
    assert!(super::text::attribution_added("  added: p0-core/94"));
}
