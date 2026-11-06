//! Cross-process FIFO build admission, warm slots and clean-tree CI receipts.
mod admission;
mod cache;
#[cfg(test)]
mod cache_tests;
mod environment;
mod queue;
mod receipt;
mod slot;
#[cfg(test)]
mod tests;

use admission::{Admission, caller_jobs, claim};
use cache::{cache, disk_free};
use environment::{configured_slots, load, memory, total_memory};
use queue::Queue;
use receipt::receipt;
use slot::{contain, prepare};

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant, SystemTime};

pub(super) use cache::prune_cache;

pub fn run(root: &Path, args: &[OsString]) -> Result<ExitCode, String> {
    let cpus = std::thread::available_parallelism().map_or(1, usize::from);
    let memory_gated = cfg!(any(target_os = "macos", target_os = "linux", windows));
    let max_slots = configured_slots(
        memory_gated,
        std::env::var_os("VIBECUT_BUILD_SLOTS").as_deref(),
        cpus,
        total_memory(),
    )
    .map_err(|error| error.to_string())?;
    run_in(
        root,
        args,
        cache()?,
        Admission {
            max_slots,
            cpus,
            memory_gated,
            caller_jobs: false,
        },
        disk_free,
        crate::ffmpeg::environment,
    )
}

fn run_in(
    root: &Path,
    args: &[OsString],
    base: PathBuf,
    mut admission: Admission,
    free_space: impl FnMut(&Path) -> Result<u64, String>,
    native: impl FnOnce(&mut Command) -> Result<(), String>,
) -> Result<ExitCode, String> {
    contain().map_err(|e| format!("build job object: {e}"))?;
    admission.caller_jobs = caller_jobs(args, std::env::var_os("CARGO_BUILD_JOBS").as_deref());
    fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let before = receipt(root, &base, args)?;
    if let Some(path) = &before
        && let Ok(text) = fs::read_to_string(path)
    {
        eprintln!("[build] CI already green for this tree: {}", text.trim());
        return Ok(ExitCode::SUCCESS);
    }
    let started = Instant::now();
    let mut queue = Queue::enter(&base.join("locks")).map_err(|e| e.to_string())?;
    let mut said = false;
    let mut slot = loop {
        if let Some(slot) =
            claim(&mut queue, root, admission, memory, load).map_err(|e| e.to_string())?
        {
            break slot;
        }
        if !said {
            eprintln!(
                "[build] waiting (FIFO; {}-slot capacity, extra builds need 30% free memory after reserving every running build's 3 GiB peak, bounded load and a 20s ramp)",
                admission.max_slots
            );
            said = true;
        }
        std::thread::sleep(Duration::from_secs(1));
    };
    let target = prepare(&base, root, slot.number, free_space)?;
    eprintln!(
        "[build] slot-{} after {:.0}s wait",
        slot.number,
        started.elapsed().as_secs_f64()
    );
    // Recheck after queueing and after completion: edits while waiting/building
    // cannot mint a receipt for a different or dirty tree.
    let before =
        before.filter(|path| receipt(root, &base, args).ok().flatten().as_ref() == Some(path));
    let mut command = Command::new(args.first().ok_or("missing build command")?);
    command
        .args(args.get(1..).unwrap_or(&[]))
        .current_dir(root)
        .env("CARGO_TARGET_DIR", target)
        .env_remove("RUSTC_WRAPPER");
    if std::env::var_os("CARGO_BUILD_JOBS").is_none() {
        command.env("CARGO_BUILD_JOBS", slot.jobs.to_string());
    }
    // The wrapper owns both the warm slot and the provisioned native
    // environment: compile-time discovery and runtime library loading must
    // agree with every command-specific xtask entry point.
    native(&mut command)?;
    let status = slot
        .execute(&mut command)
        .map_err(|e| format!("build command: {e}"))?;
    if status.success()
        && let Some(path) = before
        && receipt(root, &base, args)?.as_ref() == Some(&path)
    {
        fs::create_dir_all(base.join("ci-receipts")).map_err(|e| e.to_string())?;
        let sha = String::from_utf8(crate::checked(
            root,
            "git",
            &["rev-parse", "--short", "HEAD"],
        )?)
        .map_err(|e| e.to_string())?;
        fs::write(
            path,
            format!(
                "cargo xtask ci passed at {} ({:?})\n",
                sha.trim(),
                SystemTime::now()
            ),
        )
        .map_err(|e| e.to_string())?;
    }
    drop(slot);
    Ok(if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(
            status
                .code()
                .and_then(|code| u8::try_from(code).ok())
                .unwrap_or(1),
        )
    })
}
