//! Captured subprocess output for repository tools with typed failures.
use std::path::Path;
use std::process::{Command, Output};

pub fn output(root: &Path, program: &str, args: &[&str]) -> Result<Output, String> {
    Command::new(program)
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("{program}: {e}"))
}

pub fn checked(root: &Path, program: &str, args: &[&str]) -> Result<Vec<u8>, String> {
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
