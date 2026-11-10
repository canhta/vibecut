//! Generated-tree tests for the crate-edge, asset, name and scaffolding gates.
use super::{assets, edges, names, scaffold};
use crate::process::checked;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("target/harness-tests")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(dir.join("docs/engineering")).unwrap();
        fs::create_dir_all(dir.join("crates")).unwrap();
        fs::write(dir.join("crates/.gitkeep"), "").unwrap();
        fs::create_dir_all(dir.join("apps")).unwrap();
        fs::write(dir.join("apps/.gitkeep"), "").unwrap();
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("docs/engineering/architecture.md"),
            dir.join("docs/engineering/architecture.md"),
        )
        .unwrap();
        fs::write(dir.join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\", \"apps/*\", \"xtask\"]\nexclude = [\"vendor\"]\nresolver = \"3\"\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2024\"\nlicense = \"MIT\"\n[workspace.lints.rust]\nunsafe_code = \"forbid\"\n").unwrap();
        fs::create_dir_all(dir.join("xtask/src")).unwrap();
        fs::write(
            dir.join("xtask/Cargo.toml"),
            "[package]\nname = \"xtask\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::write(dir.join("xtask/src/lib.rs"), "//! Tooling fixture.\n").unwrap();
        Self(dir)
    }

    fn stub_crate(&self, name: &str) {
        let dir = self.0.join("vendor").join(name);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
        )
        .unwrap();
        fs::write(dir.join("src/lib.rs"), "").unwrap();
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn empty_and_allowed_workspace_edges_pass_but_aliases_do_not_hide_forbidden_edges() {
    let ws = Workspace::new();
    edges::layers(&ws.0).unwrap();
    scaffold::new_crate(&ws.0, "time").unwrap();
    scaffold::new_crate(&ws.0, "media").unwrap();
    let media = ws.0.join("crates/media/Cargo.toml");
    let original = fs::read_to_string(&media).unwrap();
    fs::write(&media, format!("{original}\n[dependencies]\nclock = {{ package = \"time\", path = \"../time\", optional = true }}\n")).unwrap();
    edges::layers(&ws.0).unwrap();
    let time = ws.0.join("crates/time/Cargo.toml");
    let original = fs::read_to_string(&time).unwrap();
    for table in [
        "dev-dependencies",
        "build-dependencies",
        "target.'cfg(windows)'.dependencies",
    ] {
        fs::write(
            &time,
            format!(
                "{original}\n[{table}]\nrenamed = {{ package = \"media\", path = \"../media\" }}\n"
            ),
        )
        .unwrap();
        assert!(edges::layers(&ws.0).unwrap_err().contains("time -> media"));
    }
}

#[test]
fn app_only_crates_are_denied_below_apps_in_every_dependency_kind() {
    let ws = Workspace::new();
    let denied = [
        ("egui", "ui"),
        ("eframe", "ui"),
        ("egui_kittest", "ui"),
        ("winit", "window"),
        ("cpal", "audio-device"),
        ("accesskit", "accessibility"),
        ("wgpu", "gpu"),
        ("rfd", "window"),
        ("native-dialog", "window"),
    ];
    for (name, _) in denied {
        ws.stub_crate(name);
    }
    scaffold::new_crate(&ws.0, "render").unwrap();
    let manifest = ws.0.join("crates/render/Cargo.toml");
    let original = fs::read_to_string(&manifest).unwrap();
    for (table, detail) in [
        ("dependencies", "kind normal"),
        ("dev-dependencies", "kind dev"),
        ("build-dependencies", "kind build"),
        (
            "target.'cfg(windows)'.dependencies",
            "kind normal, target cfg(windows)",
        ),
    ] {
        for (name, class) in denied {
            fs::write(
                &manifest,
                format!("{original}\n[{table}]\n{name} = {{ path = \"../../vendor/{name}\" }}\n"),
            )
            .unwrap();
            let error = edges::layers(&ws.0).unwrap_err();
            for expected in [
                format!("forbidden app-only {class} crate"),
                "package `render`".to_owned(),
                format!("dependency `{name}`"),
                detail.to_owned(),
            ] {
                assert!(error.contains(&expected), "{expected}: {error}");
            }
        }
    }
    fs::write(
        &manifest,
        format!("{original}\n[dependencies]\ngui = {{ package = \"egui\", path = \"../../vendor/egui\" }}\n"),
    )
    .unwrap();
    let error = edges::layers(&ws.0).unwrap_err();
    assert!(error.contains("dependency `egui`"), "{error}");
    assert!(error.contains("renamed gui"), "{error}");
}

#[test]
fn app_members_may_use_app_only_crates_and_continue_to_follow_the_table() {
    let ws = Workspace::new();
    ws.stub_crate("egui");
    ws.stub_crate("egui_kittest");
    for name in ["wgpu", "rfd", "native-dialog"] {
        ws.stub_crate(name);
    }
    scaffold::new_crate(&ws.0, "engine").unwrap();
    scaffold::new_crate(&ws.0, "time").unwrap();
    let app = ws.0.join("apps/vibecut");
    fs::create_dir_all(app.join("src")).unwrap();
    fs::write(app.join("src/main.rs"), "fn main() {}\n").unwrap();
    let manifest = app.join("Cargo.toml");
    fs::write(
        &manifest,
        "[package]\nname = \"vibecut\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\negui = { path = \"../../vendor/egui\" }\nwgpu = { path = \"../../vendor/wgpu\" }\nrfd = { path = \"../../vendor/rfd\" }\nnative-dialog = { path = \"../../vendor/native-dialog\" }\nengine = { path = \"../../crates/engine\" }\n\n[dev-dependencies]\negui_kittest = { path = \"../../vendor/egui_kittest\" }\n",
    )
    .unwrap();
    edges::layers(&ws.0).unwrap();
    // Narrow this fixture's app contract explicitly: the real app now admits
    // every production core owner, including its integer transport clock.
    let architecture = ws.0.join("docs/engineering/architecture.md");
    let policy = fs::read_to_string(&architecture).unwrap();
    let narrowed = policy
        .lines()
        .map(|line| {
            if line.starts_with("| vibecut |") {
                let mut cells: Vec<_> = line.split('|').collect();
                if let Some(edges) = cells.get_mut(4) {
                    *edges = " engine ";
                }
                cells.join("|")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(architecture, narrowed).unwrap();

    fs::write(
        &manifest,
        "[package]\nname = \"vibecut\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\ntime = { path = \"../../crates/time\" }\n",
    )
    .unwrap();
    assert!(
        edges::layers(&ws.0)
            .unwrap_err()
            .contains("forbidden workspace edge: vibecut -> time")
    );
}

#[test]
fn dev_only_crates_are_admitted_only_in_dev_dependencies() {
    let ws = Workspace::new();
    scaffold::new_crate(&ws.0, "time").unwrap();
    scaffold::new_crate(&ws.0, "testkit").unwrap();
    let architecture = ws.0.join("docs/engineering/architecture.md");
    let policy = fs::read_to_string(&architecture).unwrap();
    let narrowed = policy
        .lines()
        .map(|line| {
            if line.starts_with("| time |") {
                let mut cells: Vec<_> = line.split('|').collect();
                if let Some(edges) = cells.get_mut(4) {
                    *edges = " testkit ";
                }
                cells.join("|")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&architecture, narrowed).unwrap();
    let manifest = ws.0.join("crates/time/Cargo.toml");
    let original = fs::read_to_string(&manifest).unwrap();
    for table in ["dependencies", "build-dependencies"] {
        fs::write(
            &manifest,
            format!("{original}\n[{table}]\ntestkit = {{ path = \"../testkit\" }}\n"),
        )
        .unwrap();
        let error = edges::layers(&ws.0).unwrap_err();
        assert!(error.contains("time -> testkit"), "{error}");
        assert!(error.contains("dev-only"), "{error}");
    }
    fs::write(
        &manifest,
        format!("{original}\n[dev-dependencies]\ntestkit = {{ path = \"../testkit\" }}\n"),
    )
    .unwrap();
    edges::layers(&ws.0).unwrap();
}

#[test]
fn integration_tests_use_one_entry_point_even_with_explicit_targets() {
    let ws = Workspace::new();
    scaffold::new_crate(&ws.0, "time").unwrap();
    let dir = ws.0.join("crates/time");
    fs::create_dir_all(dir.join("tests/it")).unwrap();
    fs::write(dir.join("tests/it/main.rs"), "mod behaviour;\n").unwrap();
    fs::write(
        dir.join("tests/it/behaviour.rs"),
        "#[test] fn behaviour() {}\n",
    )
    .unwrap();
    edges::layers(&ws.0).unwrap();
    fs::write(dir.join("tests/new_behaviour.rs"), "").unwrap();
    let error = edges::layers(&ws.0).unwrap_err();
    assert!(
        error.replace('\\', "/").contains("tests/new_behaviour.rs"),
        "{error}"
    );
    assert!(
        error.contains("move integration tests to tests/it/"),
        "{error}"
    );
    fs::remove_file(dir.join("tests/new_behaviour.rs")).unwrap();
    let manifest = dir.join("Cargo.toml");
    let original = fs::read_to_string(&manifest).unwrap();
    fs::write(dir.join("custom.rs"), "fn main() {}\n").unwrap();
    for extra in [
        "[[test]]\nname = \"custom\"\npath = \"custom.rs\"\nharness = false\n",
        "[[test]]\nname = \"duplicate\"\npath = \"tests/it/main.rs\"\n[[test]]\nname = \"second\"\npath = \"tests/it/main.rs\"\n",
    ] {
        fs::write(&manifest, format!("{original}\n{extra}")).unwrap();
        let error = edges::layers(&ws.0).unwrap_err();
        assert!(
            error.contains("move integration tests to tests/it/"),
            "{error}"
        );
    }
    fs::write(&manifest, original).unwrap();
    edges::layers(&ws.0).unwrap();
}

#[test]
fn scaffold_rejects_unknown_names_and_preserves_existing_files() {
    let ws = Workspace::new();
    assert!(scaffold::new_crate(&ws.0, "../escape").is_err());
    scaffold::new_crate(&ws.0, "time").unwrap();
    let lib = ws.0.join("crates/time/src/lib.rs");
    fs::write(&lib, "//! Existing work.\n").unwrap();
    assert!(scaffold::new_crate(&ws.0, "time").is_err());
    assert_eq!(fs::read_to_string(lib).unwrap(), "//! Existing work.\n");
}

#[test]
fn assets_require_populated_sidecars_including_nested_sources() {
    let ws = Workspace::new();
    let dir = ws.0.join("assets/icon/Assets");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("source.svg"), "<svg/>").unwrap();
    assert!(assets::check(&ws.0).is_err());
    fs::write(dir.join("source.svg.attribution"), "asset: source.svg\n").unwrap();
    assert!(assets::check(&ws.0).is_err());
    fs::write(dir.join("source.svg.attribution"), "asset: source.svg\ntitle: icon\nauthor: maintainer\nsource: original work\nlicense: MIT\nlicense-file: LICENSE\nadded: test\nnotes: synthetic fixture\n").unwrap();
    assets::check(&ws.0).unwrap();
}
#[test]
fn name_gate_checks_working_files_attribution_lines_and_branch_messages() {
    let ws = Workspace::new();
    checked(&ws.0, "git", &["init", "--initial-branch=main"]).unwrap();
    names::check(&ws.0).unwrap();
    checked(&ws.0, "git", &["config", "user.name", "Harness test"]).unwrap();
    checked(
        &ws.0,
        "git",
        &["config", "user.email", "test@example.invalid"],
    )
    .unwrap();
    fs::write(ws.0.join(".gitignore"), ".scratch/\n").unwrap();
    fs::write(ws.0.join("sample"), "clean\n").unwrap();
    checked(&ws.0, "git", &["add", "."]).unwrap();
    checked(
        &ws.0,
        "git",
        &["-c", "commit.gpgsign=false", "commit", "-m", "Initial"],
    )
    .unwrap();
    fs::create_dir_all(ws.0.join(".scratch")).unwrap();
    fs::write(
        ws.0.join(".scratch/forbidden-names.txt"),
        "\nforbidden_sentinel\n\nsecond_sentinel\n",
    )
    .unwrap();
    names::check(&ws.0).unwrap();
    fs::write(ws.0.join("sample"), "FORBIDDEN_SENTINEL\n").unwrap();
    checked(&ws.0, "git", &["add", "sample"]).unwrap();
    assert!(names::check(&ws.0).unwrap_err().contains("tracked content"));
    // Attribution `added:` lines are provenance, not a licence to leak a name.
    fs::write(ws.0.join("sample"), "added: FORBIDDEN_SENTINEL\n").unwrap();
    checked(&ws.0, "git", &["add", "sample"]).unwrap();
    assert!(names::check(&ws.0).unwrap_err().contains("tracked content"));
    fs::write(ws.0.join("sample"), "clean again\n").unwrap();
    checked(&ws.0, "git", &["checkout", "-b", "topic"]).unwrap();
    checked(&ws.0, "git", &["add", "sample"]).unwrap();
    checked(
        &ws.0,
        "git",
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "FORBIDDEN_SENTINEL",
        ],
    )
    .unwrap();
    assert!(names::check(&ws.0).unwrap_err().contains("commit message"));
}
