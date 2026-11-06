//! Run and capture FFmpeg build commands with platform compiler defaults.
use std::process::Command;

pub(super) fn run(command: &mut Command) -> Result<(), String> {
    if cfg!(windows) {
        command
            .env("CC", "clang")
            .env("CXX", "clang++")
            .env("AR", "llvm-ar")
            .env("NM", "llvm-nm");
    }
    let result = command.status().map_err(|e| e.to_string())?;
    if result.success() {
        Ok(())
    } else {
        Err(format!("{command:?}: {result}"))
    }
}

pub(super) fn capture(command: &mut Command) -> Result<String, String> {
    let result = command.output().map_err(|e| e.to_string())?;
    if !result.status.success() {
        return Err(format!(
            "{command:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    String::from_utf8(result.stdout).map_err(|e| e.to_string())
}
