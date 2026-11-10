//! Offscreen UI audit scene rendering for design-review screenshots.
use std::env;
use std::path::Path;
use std::process::Command;

/// Render the offscreen UI audit scenes. This deliberately sits outside `ci`:
/// screenshots are design-review evidence, never a build gate.
pub fn run(root: &Path, scenes: &[&str]) -> Result<(), String> {
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
    crate::ffmpeg::environment(&mut command)?;
    let status = command.status().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("cargo run --example ui_audit failed: {status}"));
    }
    Ok(())
}
