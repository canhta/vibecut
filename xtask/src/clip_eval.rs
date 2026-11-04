//! Thin launcher for the labelled clip-pipeline evaluation harness.
use crate::ffmpeg;
use std::{path::Path, process::Command};

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let mut command = Command::new(std::env::var_os("CARGO").ok_or("run via cargo xtask")?);
    command
        .current_dir(root)
        .args(["run", "-p", "clip-eval", "--"])
        .args(args);
    ffmpeg::environment(&mut command)?;
    let status = command.status().map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("clip-eval failed: {status}"))
    }
}
