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
    _lock: WriterLock,
    identity: LockIdentity,
    baseline: Option<ContentHash>,
}

struct WriterLock(File);
impl Drop for WriterLock {
    fn drop(&mut self) {
        // Closing alone retains flock while another thread's spawned child has
        // an inherited descriptor open before exec. Release at the owner boundary,
        // including acquisition failures after the lock succeeds.
        let _ = self.0.unlock();
    }
}
impl WriterLease {
    /// Nonblocking acquisition, including a not-yet-created save-as destination.
    /// Existing symlinks resolve to their target; a missing file resolves through
    /// its canonical parent. Native case/Unicode alias rules apply to the lock.
    /// Reserved writer-control names are refused, including canonical aliases.
    /// Busy, unsupported/open/lock/identity failures never permit an unlocked save.
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
        if path.file_name().is_some_and(|name| {
            name.as_encoded_bytes()
                .get(name.as_encoded_bytes().len().saturating_sub(10)..)
                .is_some_and(|suffix| suffix.eq_ignore_ascii_case(b".meta.json"))
        }) {
            return Err(Error::Unsupported(
                "destination uses the project-metadata namespace",
            ));
        }
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
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // FILE_SHARE_READ | FILE_SHARE_WRITE; omit FILE_SHARE_DELETE so the
            // open sidecar cannot be renamed/replaced while its identity is read.
            options.share_mode(0x00000001 | 0x00000002);
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
        let lock = loop {
            if cancelled() || (!first_attempt && Instant::now() >= deadline) {
                return Err(Error::Busy);
            }
            first_attempt = false;
            match lock.try_lock() {
                Ok(()) => break WriterLock(lock),
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
        };
        if cancelled() {
            return Err(Error::Busy);
        }
        let identity = held_identity(&lock.0, &lock_path).map_err(Error::LockUnavailable)?;
        let mut writer = Self {
            path,
            _lock: lock,
            identity,
            baseline: None,
        };
        writer.check_identity()?;
        // No target bytes are read until the exclusive, correctly named lease is held.
        writer.baseline = current(&writer.path)?.map(|bytes| ContentHash::from_bytes(&bytes));
        writer.check_identity()?;
        Ok(writer)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Optional presentation state under the same canonical project lease.
    pub fn metadata(&self) -> crate::Metadata {
        crate::metadata::load(&self.path)
    }

    /// Atomically publish bounded metadata, refusing a changed project or lease.
    pub fn save_metadata(&self, metadata: &crate::Metadata) -> Result<()> {
        self.checked_current()?;
        crate::metadata::save(&self.path, metadata, || self.checked_current().map(|_| ()))
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
        crate::atomic::atomic_write_checked(&self.path, &bytes, || {
            self.checked_current().map(|_| ())
        })?;
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
        self.check_identity()?;
        if resolve(destination)? == self.path {
            return self.save(project);
        }
        let mut destination = Self::acquire(destination)?;
        let receipt = self.save_to(&mut destination, project)?;
        *self = destination;
        Ok(receipt)
    }

    /// Publish to an already acquired distinct destination while retaining this
    /// source lease. Background consumers adopt/drop the destination only after
    /// acknowledging its receipt; an invalidated source cannot authorize a copy.
    pub fn save_to(&self, destination: &mut Self, project: &Project) -> Result<SaveReceipt> {
        self.check_identity()?;
        destination.save(project)
    }

    fn checked_current(&self) -> Result<Option<Vec<u8>>> {
        self.check_identity()?;
        let bytes = current(&self.path)?;
        if bytes.as_ref().map(|bytes| ContentHash::from_bytes(bytes)) != self.baseline {
            return Err(Error::Conflict);
        }
        self.check_identity()?;
        Ok(bytes)
    }
    fn check_identity(&self) -> Result<()> {
        let path = lock_path(&self.path)?;
        let metadata = fs::symlink_metadata(&path).map_err(|_| Error::Conflict)?;
        if !metadata.file_type().is_file()
            || named_identity(&path, &metadata).map_err(|_| Error::Conflict)? != self.identity
        {
            return Err(Error::Conflict);
        }
        Ok(())
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
    crate::atomic::admit_destination(path)?;
    match fs::canonicalize(path) {
        Ok(path) => {
            crate::atomic::admit_destination(&path)?;
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

#[cfg(unix)]
type LockIdentity = (u64, u64);
#[cfg(windows)]
type LockIdentity = file_id::FileId;
#[cfg(not(any(unix, windows)))]
type LockIdentity = ();

#[cfg(unix)]
fn held_identity(file: &File, _: &Path) -> std::io::Result<LockIdentity> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    Ok((metadata.dev(), metadata.ino()))
}
#[cfg(unix)]
fn named_identity(_: &Path, metadata: &fs::Metadata) -> std::io::Result<LockIdentity> {
    use std::os::unix::fs::MetadataExt;
    Ok((metadata.dev(), metadata.ino()))
}
#[cfg(windows)]
fn held_identity(_: &File, path: &Path) -> std::io::Result<LockIdentity> {
    // The file is already open without delete sharing. Do not degrade to 64-bit
    // IDs: those do not establish object identity on ReFS.
    file_id::get_high_res_file_id(path)
}
#[cfg(windows)]
fn named_identity(path: &Path, _: &fs::Metadata) -> std::io::Result<LockIdentity> {
    file_id::get_high_res_file_id(path)
}
#[cfg(not(any(unix, windows)))]
fn held_identity(_: &File, _: &Path) -> std::io::Result<LockIdentity> {
    Err(std::io::ErrorKind::Unsupported.into())
}
#[cfg(not(any(unix, windows)))]
fn named_identity(_: &Path, _: &fs::Metadata) -> std::io::Result<LockIdentity> {
    Err(std::io::ErrorKind::Unsupported.into())
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    #[allow(clippy::unwrap_used, reason = "synthetic lease descriptor fixture")]
    fn dropping_lease_releases_lock_while_duplicate_descriptor_remains_open() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("project.vibecut");
        let writer = super::WriterLease::acquire(&path).unwrap();
        // A subprocess spawned on another thread can inherit this same open
        // file description until exec. Clone pins it deterministically here.
        let inherited = writer._lock.0.try_clone().unwrap();
        assert!(matches!(
            super::WriterLease::acquire(&path),
            Err(crate::Error::Busy)
        ));
        drop(writer);
        let replacement = super::WriterLease::acquire(&path).unwrap();
        drop(inherited);
        assert!(matches!(
            super::WriterLease::acquire(&path),
            Err(crate::Error::Busy)
        ));
        drop(replacement);
        assert!(super::WriterLease::acquire(&path).is_ok());
    }
}
