//! Warm target slots: lease lifetime, owner records and worktree switch preparation.
use super::{
    cache::{remove_dir, used_bytes},
    environment::GIB,
    queue::Lock,
};
use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    time::SystemTime,
};

pub(super) struct Slot {
    pub(super) lock: Lock,
    pub(super) number: usize,
    pub(super) jobs: usize,
}

impl Slot {
    #[cfg(unix)]
    pub(super) fn execute(&mut self, command: &mut Command) -> io::Result<ExitStatus> {
        // Only this slot descriptor crosses exec. Queue/admission descriptors
        // retain CLOEXEC. Closing the launcher must not explicitly unlock the
        // shared open-file description while a child/descendant still holds it.
        self.lock.unlock_on_drop = false;
        let file = &self.lock.file;
        let flags = rustix::io::fcntl_getfd(file)?;
        rustix::io::fcntl_setfd(file, flags & !rustix::io::FdFlags::CLOEXEC)?;
        let child = command.spawn();
        // Descriptor flags are per-process; restore the parent's CLOEXEC before
        // spawning receipt commands, without changing the child's copy.
        let restored = rustix::io::fcntl_setfd(file, flags);
        // Even if flag restoration fails, wait for a successfully spawned child.
        let status = child?.wait();
        restored?;
        status
    }

    #[cfg(windows)]
    pub(super) fn execute(&mut self, command: &mut Command) -> io::Result<ExitStatus> {
        // The launcher already sits in a kill-on-close job (`contain`), so the
        // command and every descendant joined it atomically at creation. Keep
        // the lease until the launcher process ends: its teardown closes the
        // job handle (killing survivors) and the slot lock together, instead
        // of releasing the slot while a detached descendant still runs.
        self.lock.unlock_on_drop = false;
        // A leaked duplicate keeps the lock's file object open past `drop(slot)`.
        std::mem::forget(self.lock.file.try_clone()?);
        command.status()
    }
}

/// Put this launcher in a kill-on-close Job Object before it creates the slot
/// lease or any child. Descendants inherit the job at creation (no spawn/assign
/// race); however the launcher ends, including `taskkill /F`, the kernel closes
/// the job handle and kills them. Called once, early: the job handle is the
/// lowest handle this process owns, so teardown reaches it before the locks.
#[cfg(windows)]
pub(super) fn contain() -> io::Result<()> {
    use win_custody::{Job, JobLimits};
    Job::create(&JobLimits::new().kill_on_close(true))?.assign_current_process()
}

#[cfg(not(windows))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "same signature as the Windows job"
)]
pub(super) fn contain() -> io::Result<()> {
    Ok(())
}

pub(super) fn owner(locks: &Path, number: usize) -> PathBuf {
    fs::read_to_string(locks.join(format!("slot-{number}.owner")))
        .map(PathBuf::from)
        .unwrap_or_default()
}
pub(super) fn prepare(
    base: &Path,
    root: &Path,
    number: usize,
    mut free_space: impl FnMut(&Path) -> Result<u64, String>,
) -> Result<PathBuf, String> {
    let locks = base.join("locks");
    let target = base.join(format!("target/slot-{number}"));
    let switched = owner(&locks, number) != root;
    let used = if switched {
        used_bytes(&base.join("target")).map_err(|e| e.to_string())?
    } else {
        0
    };
    if used > 50 * GIB || free_space(base)? < 30 * GIB {
        eprintln!("[build] cache above 50 GiB or disk below 30 GiB free; clearing slot-{number}");
        remove_dir(&target).map_err(|e| e.to_string())?;
        // Never start a build when reclaiming this slot cannot restore headroom.
        if free_space(base)? < 30 * GIB {
            return Err("build refused: disk still below 30 GiB free after slot cleanup".into());
        }
    }
    if switched {
        // Invalidate ownership before pruning/touching so interruption retries the
        // entire switch, rather than admitting stale cross-worktree artifacts.
        let owner_file = locks.join(format!("slot-{number}.owner"));
        fs::write(&owner_file, b"").map_err(|e| e.to_string())?;
        let manifests = crate::checked(root, "git", &["ls-files", "-z", "*/Cargo.toml"])?;
        let members: Vec<_> = paths(&manifests)?
            .iter()
            .filter_map(|path| path.parent()?.file_name()?.to_str())
            .map(|name| name.replace('-', "_"))
            .collect();
        let deps = target.join("debug/deps");
        if deps.is_dir() {
            for entry in fs::read_dir(&deps).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let stem = name
                    .strip_prefix("lib")
                    .unwrap_or(&name)
                    .split('-')
                    .next()
                    .unwrap_or("");
                // ponytail: name matching also drops the third-party time rlib;
                // exact package IDs if that small rebuild becomes material.
                if name.ends_with(".o")
                    || !name.contains('.')
                    || name.ends_with(".exe")
                    || members.iter().any(|member| member == stem)
                {
                    let result = if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                        remove_dir(&entry.path())
                    } else {
                        fs::remove_file(entry.path())
                    };
                    if let Err(e) = result
                        && e.kind() != io::ErrorKind::NotFound
                    {
                        return Err(e.to_string());
                    }
                }
            }
        }
        let tracked = crate::checked(root, "git", &["ls-files", "-z"])?;
        let now = SystemTime::now();
        for path in paths(&tracked)? {
            // The launcher has a per-worktree target and is already fresh.
            // Touching it would needlessly rebuild the executing launcher in
            // a nested `cargo xtask ci`.
            if path.starts_with("xtask") {
                continue;
            }
            let path = root.join(path);
            // Ignore deleted working-tree files and symlinks, never touch a
            // symlink's target outside the checkout.
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_file() => File::options()
                    .write(true)
                    .open(&path)
                    .and_then(|file| file.set_modified(now))
                    .map_err(|e| e.to_string())?,
                Ok(_) => (),
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.to_string()),
            }
        }
        fs::write(
            owner_file,
            root.to_str().ok_or("checkout path is not UTF-8")?,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(target)
}

#[cfg(unix)]
pub(super) fn paths(bytes: &[u8]) -> Result<Vec<PathBuf>, String> {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
    Ok(bytes
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|bytes| PathBuf::from(OsStr::from_bytes(bytes)))
        .collect())
}

#[cfg(windows)]
pub(super) fn paths(bytes: &[u8]) -> Result<Vec<PathBuf>, String> {
    std::str::from_utf8(bytes)
        .map(|text| {
            text.split('\0')
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .map_err(|error| format!("Git path is not UTF-8: {error}"))
}
