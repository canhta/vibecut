//! Workspace checks and admitted crate scaffolding.
mod build;
mod checkout;
mod cleanup;
mod distribution;
mod docs;
mod ffmpeg;
mod i18n;
mod measure;
mod modules;
mod orca;
mod reports;
mod tickets;

use serde_json::Value;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, ExitCode, Output};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn ci(root: &Path, require_oracles: bool) -> Result<(), String> {
    let checks = ffmpeg::begin(require_oracles).and_then(|()| run_checks(root, require_oracles));
    let fixtures = ffmpeg::print_fixtures(root);
    checks.and(fixtures)
}

fn cargo_check(root: &Path, args: &[&str], require_oracles: bool) -> Result<(), String> {
    eprintln!("cargo {}", args.join(" "));
    let mut command = Command::new(env::var_os("CARGO").ok_or("run via cargo xtask")?);
    command.args(args).current_dir(root);
    if require_oracles {
        command.env("VIBECUT_REQUIRE_ORACLES", "1");
    }
    ffmpeg::environment(&mut command)?;
    let status = command.status().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("cargo {} failed: {status}", args.join(" ")));
    }
    Ok(())
}

fn checks(root: &Path) -> Result<(), String> {
    layers(root)?;
    assets(root)?;
    i18n::check(root)?;
    names(root)?;
    modules::check(root)?;
    docs::check(root)?;
    checked(root, env!("CARGO"), &["fmt", "--all", "--", "--check"])?;
    Ok(())
}

fn clippy(root: &Path) -> Result<(), String> {
    cargo_check(
        root,
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
        false,
    )
}

fn lint(root: &Path) -> Result<(), String> {
    checks(root)?;
    clippy(root)
}

fn tests(root: &Path, args: &[&str], required: bool) -> Result<(), String> {
    let checks = ffmpeg::begin(required).and_then(|()| cargo_check(root, args, required));
    let fixtures = ffmpeg::print_fixtures(root);
    checks.and(fixtures)
}

fn run_checks(root: &Path, require_oracles: bool) -> Result<(), String> {
    lint(root)?;
    #[cfg(windows)]
    for feature in ["whisper", "gipformer"] {
        cargo_check(
            root,
            &[
                "test",
                "-p",
                "ai",
                "--lib",
                "--no-default-features",
                "--features",
                feature,
            ],
            require_oracles,
        )?;
    }
    cargo_check(root, &["test", "--workspace"], require_oracles)
}

fn output(root: &Path, program: &str, args: &[&str]) -> Result<Output, String> {
    Command::new(program)
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("{program}: {e}"))
}

fn checked(root: &Path, program: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = output(root, program, args)?;
    if !out.status.success() {
        return Err(format!(
            "{program} {}: {}\n{}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(out.stdout)
}

fn policy(root: &Path) -> Result<BTreeMap<String, (String, Vec<String>)>, String> {
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
            if rules
                .insert((*name).into(), ((*path).into(), allowed))
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

fn layers(root: &Path) -> Result<(), String> {
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
    for p in packages.iter().filter(|p| members.contains(&p["id"])) {
        let name = p["name"].as_str().ok_or("package has no name")?;
        integration_test_layout(p)?;
        let deps = p["dependencies"]
            .as_array()
            .ok_or("package has no dependency array")?;
        let (path, allowed) = if name == "xtask" {
            ("xtask".to_owned(), Vec::new())
        } else {
            let (path, allowed) = rules
                .get(name)
                .ok_or_else(|| format!("unadmitted package: {name}"))?;
            let manifest = Path::new(p["manifest_path"].as_str().ok_or("missing manifest path")?);
            if manifest != root.join(path).join("Cargo.toml") {
                errors.push(format!("{name}: expected {path}/Cargo.toml"));
            }
            (path.clone(), allowed.clone())
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
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    eprintln!("layers: ok");
    Ok(())
}

fn assets(root: &Path) -> Result<(), String> {
    fn walk(dir: &Path) -> Result<(), String> {
        if !dir.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                walk(&path)?;
            } else if path.file_name().is_some_and(|n| n != "README.md")
                && path.extension().is_none_or(|e| e != "attribution")
            {
                let mut sidecar = path.as_os_str().to_os_string();
                sidecar.push(".attribution");
                let content = fs::read_to_string(&sidecar)
                    .map_err(|e| format!("{}: attribution missing: {e}", path.display()))?;
                for key in [
                    "asset",
                    "title",
                    "author",
                    "source",
                    "license",
                    "license-file",
                    "added",
                    "notes",
                ] {
                    if !content.lines().any(|line| {
                        line.strip_prefix(&format!("{key}:"))
                            .is_some_and(|v| !v.trim().is_empty())
                    }) {
                        return Err(format!(
                            "{}: empty/missing {key}",
                            Path::new(&sidecar).display()
                        ));
                    }
                }
            }
        }
        Ok(())
    }
    walk(&root.join("assets"))?;
    walk(&root.join("docs/brand"))?;
    eprintln!("assets: ok");
    Ok(())
}

fn names(root: &Path) -> Result<(), String> {
    let patterns = checkout::main(root)?.join(".scratch/forbidden-names.txt");
    if !patterns.exists() {
        return Ok(());
    }
    let bytes = fs::read_to_string(&patterns).map_err(|e| e.to_string())?;
    let patterns: Vec<_> = bytes.lines().filter(|s| !s.trim().is_empty()).collect();
    let args: Vec<_> = patterns.iter().flat_map(|s| ["-e", *s]).collect();
    if patterns.is_empty() {
        return Ok(());
    }
    let mut grep_args = vec!["grep", "-l", "-i", "-E"];
    grep_args.extend(&args);
    grep_args.push("--");
    let out = output(root, "git", &grep_args)?;
    match out.status.code() {
        Some(1) => (),
        Some(0) => {
            return Err(format!(
                "forbidden tracked content:\n{}",
                String::from_utf8_lossy(&out.stdout)
            ));
        }
        _ => {
            return Err(format!(
                "name check: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    let base = checked(root, "git", &["merge-base", "main", "HEAD"])?;
    let base = String::from_utf8(base).map_err(|e| e.to_string())?;
    let head = checked(root, "git", &["rev-parse", "HEAD"])?;
    let range = if base.as_bytes() == head {
        "HEAD^!".into()
    } else {
        format!("{}..HEAD", base.trim())
    };
    let mut log_args = vec![
        "log",
        "--format=%H %B",
        "--regexp-ignore-case",
        "--extended-regexp",
    ];
    let filters: Vec<_> = patterns.iter().map(|s| format!("--grep={s}")).collect();
    log_args.extend(filters.iter().map(String::as_str));
    log_args.push(&range);
    let matches = checked(root, "git", &log_args)?;
    if !matches.is_empty() {
        return Err(format!(
            "forbidden branch commit message:\n{}",
            String::from_utf8_lossy(&matches)
        ));
    }
    eprintln!("names: ok");
    Ok(())
}

/// Render the offscreen UI audit scenes. This deliberately sits outside `ci`:
/// screenshots are design-review evidence, never a build gate.
fn ui_audit(root: &Path, scenes: &[&str]) -> Result<(), String> {
    let mut command = Command::new(env::var_os("CARGO").ok_or("run via cargo xtask")?);
    command
        .args([
            "run",
            "--quiet",
            "--package",
            "vibecut",
            "--example",
            "ui_audit",
            "--",
        ])
        .args(scenes)
        .current_dir(root);
    ffmpeg::environment(&mut command)?;
    let status = command.status().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("cargo run --example ui_audit failed: {status}"));
    }
    Ok(())
}

fn new_crate(root: &Path, name: &str) -> Result<(), String> {
    let rules = policy(root)?;
    let (path, _) = rules
        .get(name)
        .ok_or("name must be admitted in architecture.md")?;
    if !path.starts_with("crates/") {
        return Err("scaffold supports core libraries only".into());
    }
    let dir = root.join(path);
    fs::create_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let result = (|| {
        fs::create_dir(dir.join("src")).map_err(|e| e.to_string())?;
        fs::write(
            dir.join("src/lib.rs"),
            format!("//! {name}: see the crate README for its contract.\n"),
        )
        .map_err(|e| e.to_string())?;
        fs::write(dir.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\nversion.workspace = true\nedition.workspace = true\nlicense.workspace = true\npublish = false\n\n[lints]\nworkspace = true\n")).map_err(|e| e.to_string())?;
        fs::write(dir.join("README.md"), format!("# {name}\n\nThe [architecture](../../docs/engineering/architecture.md#crate-map) owns this crate's responsibility.\nBefore delivery, document purpose/exclusions, public API, supported inputs, limits and verification under the [README contract](../../docs/engineering/standards.md#public-apis-and-crate-readme).\n")).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    result
}

fn run() -> Result<ExitCode, String> {
    // Runtime, not env!: shared target dirs reuse this binary across worktrees.
    let manifest_dir = env::var_os("CARGO_MANIFEST_DIR").ok_or("run via cargo xtask")?;
    let root = Path::new(&manifest_dir)
        .parent()
        .ok_or("xtask must live in workspace root")?;
    let raw: Vec<_> = env::args_os().skip(1).collect();
    if raw.first().is_some_and(|arg| arg == "build") {
        return match raw.get(1).filter(|arg| *arg == "--") {
            Some(_) if raw.len() > 2 => build::run(root, raw.get(2..).unwrap_or(&[])),
            _ => Err("usage: cargo xtask build -- COMMAND [ARGS...]".into()),
        };
    }
    let args: Vec<_> = raw
        .into_iter()
        .map(|s| {
            s.into_string()
                .map_err(|_| "command arguments must be UTF-8")
        })
        .collect::<Result<_, _>>()?;
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["dist", ..] => distribution::run(root, args.get(1..).unwrap_or(&[])),
        ["media-bench"] => measure::media(root),
        ["frontier"] => tickets::frontier(root, None),
        ["frontier", "--blocking"] => tickets::blocking(root),
        ["frontier", assignee] => tickets::frontier(root, Some(assignee)),
        ["cleanup", ..] => cleanup::run(root, args.get(1..).unwrap_or(&[])),
        ["pull-reports", ..] => reports::run(root, args.get(1..).unwrap_or(&[])),
        ["board"] => tickets::board(root, 8765),
        ["board", port] => tickets::board(root, port.parse().map_err(|_| "invalid port")?),
        ["clip-eval", ..] => measure::clip_eval(root, args.get(1..).unwrap_or(&[])),
        ["ffmpeg"] => ffmpeg::provision(None),
        ["ffmpeg", "--cache-key"] => ffmpeg::cache_key(),
        ["sherpa"] => {
            println!("{}", ffmpeg::sherpa_provision(root, env::consts::ARCH, ffmpeg::fetch_limited)?.display());
            Ok(())
        }
        ["sherpa", "--arch", arch] => {
            println!("{}", ffmpeg::sherpa_provision(root, arch, ffmpeg::fetch_limited)?.display());
            Ok(())
        }
        ["catalog"] => cargo_check(
            root,
            &["run", "-p", "engine", "--example", "control_catalog"],
            false,
        ),
        ["checks"] => checks(root),
        ["clippy"] => clippy(root),
        ["lint"] => lint(root),
        ["test"] => tests(root, &["test", "--workspace"], true),
        ["ffmpeg", "--target", target] => ffmpeg::provision(Some(target)),
        ["layers"] => layers(root),
        ["assets"] => assets(root),
        ["modules"] => modules::check(root),
        ["docs"] => {
            layers(root)?;
            assets(root)?;
            i18n::check(root)?;
            names(root)?;
            docs::check(root)
        }
        ["i18n"] => i18n::check(root),
        ["new-crate", name] => new_crate(root, name),
        ["ui-audit"] => ui_audit(root, &[]),
        ["ui-audit", scenes @ ..] => ui_audit(root, scenes),
        ["ui-compare", ..] => measure::ui_compare(root, args.get(1..).unwrap_or(&[])),
        ["ci"] => ci(root, false),
        ["ci", "--require-oracles"] => ci(root, true),
        _ => Err(
            "usage: cargo xtask <dist COMMAND [OPTIONS]|build -- COMMAND [ARGS...]|frontier [ASSIGNEE]|board [PORT]|ffmpeg [--target PLATFORM|--cache-key]|ui-audit [SCENE...]|ui-compare OURS REFERENCE [--region X,Y,W,H] [--out DIR] [--max-error F] [--max-tile-error F]|sherpa [--arch x86_64|aarch64]|media-bench|clip-eval SET [OPTIONS]|cleanup [--environment NAME] [--dry-run] TARGET...|pull-reports [--environment NAME] [--ssh HOST] [--dry-run] WORKTREE...|ci [--require-oracles]|catalog|checks|clippy|lint|test|docs|i18n|layers|assets|modules|new-crate NAME>"
                .into(),
        ),
    }.map(|()| ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::{
        assets, checked,
        ffmpeg::{Version, inventory, parse_version},
        layers, names, new_crate,
    };
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
        layers(&ws.0).unwrap();
        new_crate(&ws.0, "time").unwrap();
        new_crate(&ws.0, "media").unwrap();
        let media = ws.0.join("crates/media/Cargo.toml");
        let original = fs::read_to_string(&media).unwrap();
        fs::write(&media, format!("{original}\n[dependencies]\nclock = {{ package = \"time\", path = \"../time\", optional = true }}\n")).unwrap();
        layers(&ws.0).unwrap();
        let time = ws.0.join("crates/time/Cargo.toml");
        let original = fs::read_to_string(&time).unwrap();
        for table in [
            "dev-dependencies",
            "build-dependencies",
            "target.'cfg(windows)'.dependencies",
        ] {
            fs::write(&time, format!("{original}\n[{table}]\nrenamed = {{ package = \"media\", path = \"../media\" }}\n")).unwrap();
            assert!(layers(&ws.0).unwrap_err().contains("time -> media"));
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
        new_crate(&ws.0, "render").unwrap();
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
                    format!(
                        "{original}\n[{table}]\n{name} = {{ path = \"../../vendor/{name}\" }}\n"
                    ),
                )
                .unwrap();
                let error = layers(&ws.0).unwrap_err();
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
        let error = layers(&ws.0).unwrap_err();
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
        new_crate(&ws.0, "engine").unwrap();
        new_crate(&ws.0, "time").unwrap();
        let app = ws.0.join("apps/vibecut");
        fs::create_dir_all(app.join("src")).unwrap();
        fs::write(app.join("src/main.rs"), "fn main() {}\n").unwrap();
        let manifest = app.join("Cargo.toml");
        fs::write(
            &manifest,
            "[package]\nname = \"vibecut\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\negui = { path = \"../../vendor/egui\" }\nwgpu = { path = \"../../vendor/wgpu\" }\nrfd = { path = \"../../vendor/rfd\" }\nnative-dialog = { path = \"../../vendor/native-dialog\" }\nengine = { path = \"../../crates/engine\" }\n\n[dev-dependencies]\negui_kittest = { path = \"../../vendor/egui_kittest\" }\n",
        )
        .unwrap();
        layers(&ws.0).unwrap();
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
            layers(&ws.0)
                .unwrap_err()
                .contains("forbidden workspace edge: vibecut -> time")
        );
    }

    #[test]
    fn integration_tests_use_one_entry_point_even_with_explicit_targets() {
        let ws = Workspace::new();
        new_crate(&ws.0, "time").unwrap();
        let dir = ws.0.join("crates/time");
        fs::create_dir_all(dir.join("tests/it")).unwrap();
        fs::write(dir.join("tests/it/main.rs"), "mod behaviour;\n").unwrap();
        fs::write(
            dir.join("tests/it/behaviour.rs"),
            "#[test] fn behaviour() {}\n",
        )
        .unwrap();
        layers(&ws.0).unwrap();
        fs::write(dir.join("tests/new_behaviour.rs"), "").unwrap();
        let error = layers(&ws.0).unwrap_err();
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
            let error = layers(&ws.0).unwrap_err();
            assert!(
                error.contains("move integration tests to tests/it/"),
                "{error}"
            );
        }
        fs::write(&manifest, original).unwrap();
        layers(&ws.0).unwrap();
    }

    #[test]
    fn scaffold_rejects_unknown_names_and_preserves_existing_files() {
        let ws = Workspace::new();
        assert!(new_crate(&ws.0, "../escape").is_err());
        new_crate(&ws.0, "time").unwrap();
        let lib = ws.0.join("crates/time/src/lib.rs");
        fs::write(&lib, "//! Existing work.\n").unwrap();
        assert!(new_crate(&ws.0, "time").is_err());
        assert_eq!(fs::read_to_string(lib).unwrap(), "//! Existing work.\n");
    }

    #[test]
    fn assets_require_populated_sidecars_including_nested_sources() {
        let ws = Workspace::new();
        let dir = ws.0.join("assets/icon/Assets");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("source.svg"), "<svg/>").unwrap();
        assert!(assets(&ws.0).is_err());
        fs::write(dir.join("source.svg.attribution"), "asset: source.svg\n").unwrap();
        assert!(assets(&ws.0).is_err());
        fs::write(dir.join("source.svg.attribution"), "asset: source.svg\ntitle: icon\nauthor: maintainer\nsource: original work\nlicense: MIT\nlicense-file: LICENSE\nadded: test\nnotes: synthetic fixture\n").unwrap();
        assets(&ws.0).unwrap();
    }
    #[test]
    fn name_gate_checks_working_files_and_branch_messages_and_skips_missing_policy() {
        let ws = Workspace::new();
        checked(&ws.0, "git", &["init", "--initial-branch=main"]).unwrap();
        names(&ws.0).unwrap();
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
        names(&ws.0).unwrap();
        fs::write(ws.0.join("sample"), "FORBIDDEN_SENTINEL\n").unwrap();
        assert!(names(&ws.0).unwrap_err().contains("tracked content"));
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
        assert!(names(&ws.0).unwrap_err().contains("commit message"));
    }

    #[test]
    fn oracle_versions_and_fixture_inventory_are_explicit() {
        assert_eq!(
            parse_version("ffmpeg version 7.1.1 Copyright"),
            Some(Version { major: 7, minor: 1 })
        );
        assert_eq!(
            parse_version("ffmpeg version n6.0-static"),
            Some(Version { major: 6, minor: 0 })
        );
        assert_eq!(parse_version("ffmpeg version N-123-gabc"), None);
        assert_eq!(parse_version("not a version"), None);
        let ws = Workspace::new();
        let fixtures = ws.0.join("target/fixtures/nested");
        fs::create_dir_all(&fixtures).unwrap();
        fs::write(fixtures.join("one.bin"), b"abc").unwrap();
        let list = inventory(&ws.0).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].relative, PathBuf::from("nested/one.bin"));
        assert_eq!(list[0].bytes, 3);
        assert_eq!(
            list[0].sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
