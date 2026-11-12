//! Supervises bounded external tools through capture files, deadlines, child teardown.
#![allow(
    clippy::indexing_slicing,
    reason = "an offline audit capture indexes its own manifest and fixed-size frame buffers; a panic aborts the tool, never a person's project"
)]
use super::super::Failure;
use super::source::TOOL_METADATA_TIMEOUT;
use super::source::TOOL_STDERR_CAP;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command as ProcessCommand;
use std::process::ExitStatus;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

#[derive(Debug)]
pub(crate) struct SupervisedOutput {
    pub(super) status: ExitStatus,
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
}

pub(super) struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn child(&mut self) -> Result<&mut Child, Failure> {
        self.0
            .as_mut()
            .ok_or_else(|| "supervised child missing".into())
    }

    fn kill(&mut self) {
        if let Some(mut child) = self.0.take() {
            child.kill().ok();
            child.wait().ok();
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

pub(super) fn read_capped(path: &Path, cap: usize, label: &str) -> Result<Vec<u8>, Failure> {
    let mut file = fs::File::open(path)?;
    let mut out = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let room = (cap + 1).saturating_sub(out.len());
        if room > 0 {
            out.extend_from_slice(&buffer[..read.min(room)]);
        }
        if out.len() > cap {
            return Err(format!("{label} output exceeded its bounded cap").into());
        }
    }
    Ok(out)
}

pub(super) fn tool_command(program: &str) -> ProcessCommand {
    let mut command = ProcessCommand::new(program);
    command
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("LD_LIBRARY_PATH");
    if let (Some(prefix), Some(paths)) = (std::env::var_os("FFMPEG_DIR"), std::env::var_os("PATH"))
    {
        let production = PathBuf::from(prefix).join("bin");
        let paths: Vec<_> = std::env::split_paths(&paths)
            .filter(|path| path != &production)
            .collect();
        if let Ok(path) = std::env::join_paths(paths) {
            command.env("PATH", path);
        }
    }
    command
}

pub(crate) fn run_tool(
    program: &str,
    args: &[&str],
    timeout: Duration,
    stdout_cap: usize,
) -> Result<SupervisedOutput, Failure> {
    let scratch = tempfile::tempdir()?;
    let out_path = scratch.path().join("stdout");
    let err_path = scratch.path().join("stderr");
    let out_file = fs::File::create(&out_path)?;
    let err_file = fs::File::create(&err_path)?;
    let child = tool_command(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file))
        .spawn()
        .map_err(|error| format!("spawn {program}: {error}"))?;
    let mut guard = ChildGuard(Some(child));
    let deadline = Instant::now() + timeout;
    let mut status = None;
    while Instant::now() < deadline {
        if let Some(exit) = guard.child()?.try_wait()? {
            status = Some(exit);
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let timed_out = status.is_none();
    if timed_out {
        guard.kill();
    }
    let stdout = read_capped(&out_path, stdout_cap, program)?;
    let stderr = read_capped(&err_path, TOOL_STDERR_CAP, program)?;
    if timed_out {
        return Err(format!("{program} timed out after {timeout:?}").into());
    }
    guard.0.take();
    Ok(SupervisedOutput {
        status: status.ok_or("supervised child status")?,
        stdout,
        stderr,
    })
}

pub(super) fn tool_identity(tool: &str) -> Result<String, Failure> {
    let output = run_tool(tool, &["-version"], TOOL_METADATA_TIMEOUT, TOOL_STDERR_CAP)?;
    if !output.status.success() {
        return Err(format!("{tool} -version failed").into());
    }
    Ok(String::from_utf8(output.stdout)?
        .lines()
        .next()
        .unwrap_or("")
        .to_owned())
}

pub(super) fn git_head(root: &Path) -> Result<String, Failure> {
    let root = root.to_string_lossy().into_owned();
    let output = run_tool(
        "git",
        &["-C", &root, "rev-parse", "HEAD"],
        TOOL_METADATA_TIMEOUT,
        TOOL_STDERR_CAP,
    )?;
    if !output.status.success() {
        return Err("git rev-parse HEAD failed".into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

pub(super) fn free_bytes(path: &Path) -> Result<u64, Failure> {
    let path = path.to_string_lossy().into_owned();
    #[cfg(windows)]
    let output = run_tool(
        "powershell",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!(
                "(Get-Item -LiteralPath '{}').PSDrive.Free",
                path.replace('\'', "''")
            ),
        ],
        TOOL_METADATA_TIMEOUT,
        TOOL_STDERR_CAP,
    )?;
    #[cfg(unix)]
    let output = run_tool(
        "df",
        &["-k", "-P", &path],
        TOOL_METADATA_TIMEOUT,
        TOOL_STDERR_CAP,
    )?;
    if !output.status.success() {
        return Err("scratch headroom probe failed".into());
    }
    let text = String::from_utf8(output.stdout)?;
    #[cfg(windows)]
    let bytes = text.trim().parse::<u64>()?;
    #[cfg(unix)]
    let bytes = text
        .lines()
        .nth(1)
        .and_then(|line| line.split_whitespace().nth(3))
        .and_then(|field| field.parse::<u64>().ok())
        .map(|kib| kib.saturating_mul(1024))
        .ok_or("df free space parse")?;
    Ok(bytes)
}
