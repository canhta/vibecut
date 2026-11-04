use std::env;
use std::path::Path;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    if args.next().as_deref() != Some("ci") || args.next().is_some() {
        return Err("usage: cargo xtask ci".into());
    }

    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("xtask must live in the workspace root")?;
    let cargo = env::var_os("CARGO").ok_or("run xtask through cargo xtask ci")?;
    for args in [
        &["fmt", "--all", "--", "--check"][..],
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
        &["test", "--workspace"],
    ] {
        eprintln!("cargo {}", args.join(" "));
        let status = Command::new(&cargo)
            .args(args)
            .current_dir(root)
            .status()
            .map_err(|error| format!("could not run cargo {}: {error}", args.join(" ")))?;
        if !status.success() {
            return Err(format!("cargo {} failed: {status}", args.join(" ")));
        }
    }
    Ok(())
}
