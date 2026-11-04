//! One adjacent OS lease plus a full-content precondition for project publication.
use crate::{Error, Loaded, Project, Result, persistence::read_bytes};
use media::ContentHash;
use std::{
    ffi::OsString,
    fs::{self, File, TryLockError},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// Successful publication of these exact bytes to this canonical native path.
/// Engine binds its own revision to this receipt before acknowledging a save.
pub struct SaveReceipt {
    path: PathBuf,
    hash: ContentHash,
}
impl SaveReceipt {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn content_hash(&self) -> ContentHash {
        self.hash
    }
}
impl std::fmt::Debug for SaveReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveReceipt")
            .field("hash", &self.hash)
            .finish_non_exhaustive()
    }
}

/// Acquired before reading a writable project; retain through editing and saving.
/// The OS releases the lease at drop/process death. The adjacent lock file stays:
/// deleting it would allow different open files to acquire independent locks.
/// This is an advisory protocol between cooperating writers, not an atomic CAS
/// against arbitrary applications that race the final check and replacement.
pub struct WriterLease {
    path: PathBuf,
    lock: File,
    baseline: Option<ContentHash>,
}
impl WriterLease {
    /// Nonblocking acquisition, including a not-yet-created save-as destination.
    /// Existing symlinks resolve to their target; a missing file resolves through
    /// its canonical parent. Native case/Unicode alias rules apply to the lock.
    /// Busy, unsupported/open/lock failures never permit an unlocked save.
    pub fn acquire(path: &Path) -> Result<Self> {
        Self::acquire_wait(path, Duration::ZERO, &|| false)
    }

    /// Poll for at most `wait`, checking cancellation between nonblocking lock
    /// attempts. Timeout/cancellation returns Busy, without reading/writing project
    /// data. Call off the UI thread; individual filesystem calls are not interruptible.
    pub fn acquire_wait(path: &Path, wait: Duration, cancelled: &dyn Fn() -> bool) -> Result<Self> {
        if cancelled() {
            return Err(Error::Busy);
        }
        let deadline = Instant::now()
            .checked_add(wait)
            .ok_or(Error::Invalid("lease wait duration"))?;
        let path = resolve(path)?;
        let lock_path = lock_path(&path)?;
        match fs::symlink_metadata(&lock_path) {
            Ok(m) if !m.file_type().is_file() => {
                return Err(Error::Unsupported("writer lock is not a regular file"));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::LockUnavailable(e)),
        }
        let mut options = File::options();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(&lock_path).map_err(Error::LockUnavailable)?;
        if !fs::symlink_metadata(&lock_path)
            .map_err(Error::LockUnavailable)?
            .file_type()
            .is_file()
            || !lock.metadata().map_err(Error::LockUnavailable)?.is_file()
        {
            return Err(Error::Unsupported("writer lock is not a regular file"));
        }
        let mut first_attempt = true;
        loop {
            if cancelled() || (!first_attempt && Instant::now() >= deadline) {
                return Err(Error::Busy);
            }
            first_attempt = false;
            match lock.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(
                        deadline
                            .saturating_duration_since(Instant::now())
                            .min(Duration::from_millis(5)),
                    );
                }
                Err(TryLockError::WouldBlock) => return Err(Error::Busy),
                Err(TryLockError::Error(error)) => return Err(Error::LockUnavailable(error)),
            }
        }
        if cancelled() {
            return Err(Error::Busy);
        }
        // No target bytes are read until the exclusive lease is held.
        let baseline = current(&path)?.map(|bytes| ContentHash::from_bytes(&bytes));
        Ok(Self {
            path,
            lock,
            baseline,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load only bytes matching the acquisition/save baseline. A noncooperating
    /// replacement is a Conflict rather than silently refreshing that baseline.
    pub fn load(&self) -> Result<Loaded> {
        let bytes = self
            .checked_current()?
            .ok_or_else(|| Error::Io(std::io::Error::from(std::io::ErrorKind::NotFound)))?;
        crate::decode(&bytes)
    }

    /// Encode, check the full saved content, then use the existing atomic writer.
    /// On refusal/failure the baseline is retained and no save receipt is issued.
    pub fn save(&mut self, project: &Project) -> Result<SaveReceipt> {
        let bytes = crate::encode(project, true)?;
        self.checked_current()?;
        crate::atomic_write(&self.path, &bytes)?;
        let hash = ContentHash::from_bytes(&bytes);
        self.baseline = Some(hash);
        Ok(SaveReceipt {
            path: self.path.clone(),
            hash,
        })
    }

    /// Acquire the destination before any save-as write. Failure retains this
    /// lease/path and source bytes. Success retains the destination lease instead.
    pub fn save_as(&mut self, destination: &Path, project: &Project) -> Result<SaveReceipt> {
        if resolve(destination)? == self.path {
            return self.save(project);
        }
        let mut destination = Self::acquire(destination)?;
        let receipt = destination.save(project)?;
        *self = destination;
        Ok(receipt)
    }

    fn checked_current(&self) -> Result<Option<Vec<u8>>> {
        // A removed lock path is never recreated while this lease is held.
        if !fs::symlink_metadata(lock_path(&self.path)?)?
            .file_type()
            .is_file()
            || !self.lock.metadata()?.is_file()
        {
            return Err(Error::Conflict);
        }
        let bytes = current(&self.path)?;
        if bytes.as_ref().map(|bytes| ContentHash::from_bytes(bytes)) != self.baseline {
            return Err(Error::Conflict);
        }
        Ok(bytes)
    }
}
impl std::fmt::Debug for WriterLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriterLease")
            .field("exists", &self.baseline.is_some())
            .finish_non_exhaustive()
    }
}
fn resolve(path: &Path) -> Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(path) => {
            if !path.is_file() {
                return Err(Error::Unsupported(
                    "writer destination is not a regular file",
                ));
            }
            Ok(path)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // A dangling symlink must not become a new target via create.
            if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(Error::Unsupported("dangling project symlink"));
            }
            let name = path.file_name().ok_or(Error::Invalid("project filename"))?;
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            Ok(fs::canonicalize(parent)?.join(name))
        }
        Err(error) => Err(error.into()),
    }
}
fn lock_path(path: &Path) -> Result<PathBuf> {
    let mut name = OsString::from(".");
    name.push(path.file_name().ok_or(Error::Invalid("project filename"))?);
    name.push(".lock");
    Ok(path.with_file_name(name))
}
fn current(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_file() => Err(Error::Conflict),
        Ok(_) => Ok(Some(read_bytes(path)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
