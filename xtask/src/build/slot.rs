//! Concurrency leases, checkout locks and child process lifetime.
use super::queue::Lock;
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

pub(super) struct Slot {
    pub(super) lock: Lock,
    pub(super) number: usize,
    pub(super) jobs: usize,
    pub(super) target_lock: Option<Lock>,
}

impl Slot {
    #[cfg(unix)]
    pub(super) fn execute(&mut self, command: &mut Command) -> io::Result<ExitStatus> {
        // Only slot and checkout descriptors cross exec. Queue/admission descriptors
        // retain CLOEXEC. Closing the launcher must not explicitly unlock the
        // shared open-file description while a child/descendant still holds it.
        let mut descriptors = Vec::new();
        for lock in [Some(&mut self.lock), self.target_lock.as_mut()]
            .into_iter()
            .flatten()
        {
            lock.unlock_on_drop = false;
            let flags = rustix::io::fcntl_getfd(&lock.file)?;
            rustix::io::fcntl_setfd(&lock.file, flags & !rustix::io::FdFlags::CLOEXEC)?;
            descriptors.push((&lock.file, flags));
        }
        let child = command.spawn();
        let restored = descriptors
            .into_iter()
            .try_for_each(|(file, flags)| rustix::io::fcntl_setfd(file, flags));
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
        for lock in [Some(&mut self.lock), self.target_lock.as_mut()]
            .into_iter()
            .flatten()
        {
            lock.unlock_on_drop = false;
            std::mem::forget(lock.file.try_clone()?);
        }
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
