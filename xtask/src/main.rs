//! Workspace checks and admitted crate scaffolding.
mod build;
mod checkout;
mod ci;
mod distribution;
mod ffmpeg;
mod i18n;
mod measure;
mod policy;
mod process;
mod ui_audit;

use process::checked;
use std::env;
use std::path::Path;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
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
    policy::check(root)?;
    i18n::check(root)?;
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
        ["build-cap", "--json"] => build::capacity(root),
        ["prune"] => build::prune(root, false, false),
        ["prune", "--dry-run"] => build::prune(root, true, false),
        ["prune", "--make-room"] => build::prune(root, false, true),
        ["prune", "--make-room", "--dry-run"] | ["prune", "--dry-run", "--make-room"] => build::prune(root, true, true),
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
        ["test"] => ci::suite(root, true),
        ["ffmpeg", "--target", target] => ffmpeg::provision(Some(target)),
        ["layers"] => policy::edges::layers(root),
        ["assets"] => policy::assets::check(root),
        ["modules"] => policy::modules::check(root),
        ["docs"] => {
            policy::edges::layers(root)?;
            policy::assets::check(root)?;
            i18n::check(root)?;
            policy::names::check(root)?;
            policy::docs::check(root)
        }
        ["i18n"] => i18n::check(root),
        ["new-crate", name] => policy::scaffold::new_crate(root, name),
        ["ui-audit"] => ui_audit::run(root, &[]),
        ["ui-audit", scenes @ ..] => ui_audit::run(root, scenes),
        ["ui-compare", ..] => measure::ui_compare(root, args.get(1..).unwrap_or(&[])),
        ["ci"] => ci::run(root, false),
        ["ci", "--require-oracles"] => ci::run(root, true),
        _ => Err(
            "usage: cargo xtask <dist COMMAND [OPTIONS]|build -- COMMAND [ARGS...]|build-cap --json|prune [--make-room] [--dry-run]|ffmpeg [--target PLATFORM|--cache-key]|ui-audit [SCENE...]|ui-compare OURS REFERENCE [--region X,Y,W,H] [--out DIR] [--max-error F] [--max-tile-error F]|sherpa [--arch x86_64|aarch64]|media-bench|clip-eval SET [OPTIONS]|ci [--require-oracles]|catalog|checks|clippy|lint|test|docs|i18n|layers|assets|modules|new-crate NAME>"
                .into(),
        ),
    }.map(|()| ExitCode::SUCCESS)
}
