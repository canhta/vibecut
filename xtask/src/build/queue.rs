//! OS-locked FIFO build queue and admission locks with crash-safe liveness.
use std::{fs::{self, File, TryLockError}, io, path::{Path, PathBuf}};

pub(super) struct Lock {
    pub(super) file: File,
    pub(super) unlock_on_drop: bool,
}

impl Lock {
    pub(super)     fn new(file: File) -> Self {
        Self {
            file,
            unlock_on_drop: true,
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        if self.unlock_on_drop {
            let _ = self.file.unlock();
        }
    }
}

pub(super) fn open(path: &Path) -> io::Result<File> {
    File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

pub(super) fn try_lock(file: File) -> io::Result<Option<Lock>> {
    match file.try_lock() {
        Ok(()) => Ok(Some(Lock::new(file))),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

pub(super) struct Queue {
    pub(super) locks: PathBuf,
    pub(super) path: PathBuf,
    pub(super) held: Option<Lock>,
}

impl Queue {
    pub(super)     fn enter(locks: &Path) -> io::Result<Self> {
        fs::create_dir_all(locks.join("build-queue"))?;
        // Serialize creation + locking, dead-entry pruning, and admission. No
        // observer can mistake a newly created, not-yet-locked waiter for dead.
        let admission = Self::admission(locks)?;
        // Derive the next number under the admission lock. No mutable counter
        // can be truncated by a crash, and no wall-clock adjustment reorders it.
        let mut largest = 0u64;
        for entry in fs::read_dir(locks.join("build-queue"))? {
            let name = entry?.file_name();
            let number = name
                .to_str()
                .ok_or_else(|| io::Error::other("invalid queue filename"))?
                .parse::<u64>()
                .map_err(io::Error::other)?;
            largest = largest.max(number);
        }
        let next = largest
            .checked_add(1)
            .ok_or_else(|| io::Error::other("build queue exhausted"))?;
        let path = locks.join("build-queue").join(format!("{next:020}"));
        let file = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.lock()?;
        drop(admission);
        Ok(Self {
            locks: locks.to_path_buf(),
            path,
            held: Some(Lock::new(file)),
        })
    }

    pub(super)     fn admission(locks: &Path) -> io::Result<Lock> {
        let file = open(&locks.join("admission"))?;
        file.lock()?;
        Ok(Lock::new(file))
    }

    pub(super)     fn first(&self) -> io::Result<bool> {
        let mut paths = fs::read_dir(self.locks.join("build-queue"))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<io::Result<Vec<_>>>()?;
        paths.sort();
        for path in paths {
            if path == self.path {
                return Ok(true);
            }
            // A held lock is the liveness signal. PIDs are
            // neither used nor trusted; a crashed waiter leaves an unlocked file.
            let file = File::options().read(true).write(true).open(&path)?;
            match try_lock(file)? {
                None => return Ok(false),
                Some(dead) => {
                    drop(dead);
                    fs::remove_file(path)?;
                }
            }
        }
        Err(io::Error::other("build queue entry disappeared"))
    }

    pub(super)     fn leave(&mut self) -> io::Result<()> {
        // Close before unlink. Caller holds admission.
        drop(self.held.take());
        fs::remove_file(&self.path)
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        if self.held.is_some()
            && let Ok(_admission) = Self::admission(&self.locks)
        {
            let _ = self.leave();
        }
    }
}
