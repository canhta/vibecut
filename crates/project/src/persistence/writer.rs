//! One adjacent OS lease plus a full-content precondition for project publication.
use super::atomic;
use super::identity::{Identity, held_identity, named_identity};
use super::metadata;
use super::read_bytes;
use crate::{Error, Loaded, Project, Result};
use media::ContentHash;
use std::{
    ffi::OsString,
    fs::{self, File, TryLockError},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// Successful publication of these exact bytes to this canonical native path.
/// Engine binds its own revision to this receipt before acknowledging a save.
pub struct SaveReceipt {
    path: PathBuf,
    hash: ContentHash,
    project_id: crate::ProjectId,
}
impl SaveReceipt {
    pub fn project_id(&self) -> crate::ProjectId {
        self.project_id
    }
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
    identity: Identity,
    baseline: Option<ContentHash>,
}

struct WriterLock(File);
const DIRECTORY_MOVES: &str = ".vibecut-directory-moves.lock";
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
        Self::acquire_wait_with(path, wait, cancelled, || {})
    }

    fn acquire_wait_with(
        path: &Path,
        wait: Duration,
        cancelled: &dyn Fn() -> bool,
        before_lock: impl FnOnce(),
    ) -> Result<Self> {
        if cancelled() {
            return Err(Error::Busy);
        }
        let deadline = Instant::now()
            .checked_add(wait)
            .ok_or(Error::Invalid("lease wait duration"))?;
        let path = resolve(path)?;
        // A managed library keeps this permanent barrier outside its movable
        // children. Retain shared admission until the adjacent lease is locked.
        let _admission = directory_admission(&path)?;
        if resolve(&path)? != path {
            return Err(Error::Conflict);
        }
        before_lock();
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
        // An acquisition begun before a library first installed its barrier
        // must not slip through a subsequent move's handle-close interval.
        let _late_admission = if _admission.is_none() {
            directory_admission(&path)?
        } else {
            None
        };
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

    /// Enable sibling-directory moves beneath an existing private library root.
    /// Call before publishing projects. The empty control file must never be
    /// removed/replaced; ordinary WriterLease acquisition automatically honors it.
    pub fn enable_directory_moves(root: &Path) -> Result<()> {
        let path = root.canonicalize()?.join(DIRECTORY_MOVES);
        let lock = open_control(&path, true)?;
        lock.try_lock().map_err(lock_error)?;
        let _lock = WriterLock(lock);
        Ok(())
    }

    /// Consume this project lease to atomically move its containing directory
    /// to an absent sibling. Requires enable_directory_moves on their common root.
    /// A root-level exclusive barrier excludes source/destination acquisition
    /// while the adjacent Windows handle is closed and through the rename.
    /// Busy, conflict or IO errors leave history publication to the caller.
    pub fn move_directory(self, destination: &Path) -> Result<()> {
        self.move_directory_with(destination, || {})
    }

    fn move_directory_with(self, destination: &Path, mut boundary: impl FnMut()) -> Result<()> {
        let source = self
            .path
            .parent()
            .ok_or(Error::Invalid("project directory"))?
            .to_owned();
        let root = source.parent().ok_or(Error::Invalid("library root"))?;
        if destination
            .parent()
            .ok_or(Error::Invalid("move destination"))?
            .canonicalize()?
            != root
        {
            return Err(Error::Unsupported("project move must stay in its library"));
        }
        atomic::admit_destination(destination)?;
        let gate_path = root.join(DIRECTORY_MOVES);
        let gate = open_control(&gate_path, false)?;
        gate.try_lock().map_err(lock_error)?;
        let gate = WriterLock(gate);
        let identity = held_identity(&gate.0, &gate_path).map_err(Error::LockUnavailable)?;
        self.validate()?;
        match fs::symlink_metadata(destination) {
            Ok(_) => return Err(Error::Conflict),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        boundary();
        drop(self);
        boundary();
        check_control(&gate.0, &gate_path, identity)?;
        fs::rename(source, destination)?;
        boundary();
        Ok(())
    }

    /// Validate the held lease identity and unchanged content, including absent targets.
    pub fn validate(&self) -> Result<()> {
        self.checked_current().map(|_| ())
    }

    /// Optional presentation state under the same canonical project lease.
    pub fn metadata(&self) -> crate::Metadata {
        metadata::load(&self.path)
    }

    /// Atomically publish bounded metadata, refusing a changed project or lease.
    pub fn save_metadata(&self, metadata: &crate::Metadata) -> Result<()> {
        self.checked_current()?;
        metadata::save(&self.path, metadata, || self.checked_current().map(|_| ()))
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
        atomic::stage_validated(
            &self.path,
            |file| file.write_all(&bytes).map_err(Error::from),
            |_| self.checked_current().map(|_| ()),
        )?;
        let hash = ContentHash::from_bytes(&bytes);
        self.baseline = Some(hash);
        Ok(SaveReceipt {
            path: self.path.clone(),
            hash,
            project_id: project.data().id,
        })
    }

    /// Acquire the destination before any save-as write. Failure retains this
    /// lease/path and source bytes. A distinct destination gets a fresh project
    /// identity, returned in the receipt; aliases retain the current identity.
    /// Success retains the destination lease instead.
    pub fn save_as(&mut self, destination: &Path, project: &Project) -> Result<SaveReceipt> {
        self.check_identity()?;
        if resolve(destination)? == self.path {
            return self.save(project);
        }
        let mut destination = Self::acquire(destination)?;
        let receipt = self.save_to(&mut destination, &project.fork()?)?;
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

fn lock_error(error: TryLockError) -> Error {
    match error {
        TryLockError::WouldBlock => Error::Busy,
        TryLockError::Error(error) => Error::LockUnavailable(error),
    }
}

fn open_control(path: &Path, create: bool) -> Result<File> {
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_file() => return Err(Error::Conflict),
        Ok(_) => {}
        Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(Error::LockUnavailable(error)),
    }
    let mut options = File::options();
    options
        .read(true)
        .write(true)
        .create(create)
        .truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0x00000001 | 0x00000002);
    }
    let file = options.open(path).map_err(Error::LockUnavailable)?;
    let identity = held_identity(&file, path).map_err(Error::LockUnavailable)?;
    check_control(&file, path, identity)?;
    Ok(file)
}

fn check_control(file: &File, path: &Path, identity: Identity) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(Error::LockUnavailable)?;
    if !metadata.file_type().is_file()
        || !file.metadata().map_err(Error::LockUnavailable)?.is_file()
        || named_identity(path, &metadata).map_err(Error::LockUnavailable)? != identity
    {
        return Err(Error::Conflict);
    }
    Ok(())
}

fn directory_admission(path: &Path) -> Result<Option<WriterLock>> {
    let Some(root) = path.parent().and_then(Path::parent) else {
        return Ok(None);
    };
    let gate_path = root.join(DIRECTORY_MOVES);
    match fs::symlink_metadata(&gate_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::LockUnavailable(error)),
        Ok(_) => {}
    }
    let gate = open_control(&gate_path, false)?;
    gate.try_lock_shared().map_err(lock_error)?;
    let gate = WriterLock(gate);
    let identity = held_identity(&gate.0, &gate_path).map_err(Error::LockUnavailable)?;
    check_control(&gate.0, &gate_path, identity)?;
    Ok(Some(gate))
}
impl std::fmt::Debug for WriterLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriterLease")
            .field("exists", &self.baseline.is_some())
            .finish_non_exhaustive()
    }
}
fn resolve(path: &Path) -> Result<PathBuf> {
    atomic::admit_destination(path)?;
    match fs::canonicalize(path) {
        Ok(path) => {
            atomic::admit_destination(&path)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(
        clippy::unwrap_used,
        reason = "gated first-open admission race fixture"
    )]
    fn writer_started_before_barrier_installation_cannot_enter_a_move() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir(&source).unwrap();
        let path = source.join("project.vibecut");
        let mut writer = WriterLease::acquire(&path).unwrap();
        writer
            .save(&Project::empty(crate::ProjectId(1), "retained".into()).unwrap())
            .unwrap();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let pending = std::thread::spawn(move || {
            let result = WriterLease::acquire_wait_with(&path, Duration::ZERO, &|| false, || {
                ready_tx.send(()).unwrap();
                resume_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            });
            result_tx.send(matches!(result, Err(Error::Busy))).unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        WriterLease::enable_directory_moves(root.path()).unwrap();
        let mut phase = 0;
        writer
            .move_directory_with(&root.path().join(".undo-copy"), || {
                phase += 1;
                if phase == 2 {
                    resume_tx.send(()).unwrap();
                    assert!(result_rx.recv_timeout(Duration::from_secs(10)).unwrap());
                }
            })
            .unwrap();
        pending.join().unwrap();
    }

    #[test]
    #[allow(clippy::unwrap_used, reason = "synthetic move-boundary fixtures")]
    fn directory_move_excludes_writers_before_close_through_publication() {
        for target in [".undo-copy", ".delete-copy"] {
            let root = tempfile::tempdir().unwrap();
            WriterLease::enable_directory_moves(root.path()).unwrap();
            let source = root.path().join("source");
            fs::create_dir(&source).unwrap();
            let path = source.join("project.vibecut");
            let mut writer = WriterLease::acquire(&path).unwrap();
            let project = Project::empty(crate::ProjectId(1), "original".into()).unwrap();
            writer.save(&project).unwrap();
            let destination = root.path().join(target);
            let mut phases = 0;
            writer
                .move_directory_with(&destination, || {
                    phases += 1;
                    assert!(WriterLease::acquire(&path).is_err());
                    assert!(WriterLease::acquire(&destination.join("project.vibecut")).is_err());
                    if phases < 3 {
                        assert!(matches!(WriterLease::acquire(&path), Err(Error::Busy)));
                    } else {
                        assert!(matches!(
                            WriterLease::acquire(&destination.join("project.vibecut")),
                            Err(Error::Busy)
                        ));
                    }
                })
                .unwrap();
            assert_eq!(phases, 3);
            assert!(!source.exists());
            assert_eq!(
                WriterLease::acquire(&destination.join("project.vibecut"))
                    .unwrap()
                    .load()
                    .unwrap()
                    .project,
                project
            );
        }
    }

    #[test]
    #[allow(clippy::unwrap_used, reason = "synthetic interrupted-move fixtures")]
    fn interrupted_directory_move_keeps_complete_reacquirable_project() {
        const CHILD_ROOT: &str = "VIBECUT_MOVE_CRASH_ROOT";
        const CHILD_PHASE: &str = "VIBECUT_MOVE_CRASH_PHASE";
        if let Some(root) = std::env::var_os(CHILD_ROOT) {
            let root = PathBuf::from(root);
            let interruption: u32 = std::env::var(CHILD_PHASE).unwrap().parse().unwrap();
            let writer = WriterLease::acquire(&root.join("source/project.vibecut")).unwrap();
            let mut phase = 0;
            writer
                .move_directory_with(&root.join(".undo-copy"), || {
                    phase += 1;
                    if phase == interruption {
                        // Abrupt exit: no Rust destructors release either lock.
                        std::process::exit(77);
                    }
                })
                .unwrap();
            return;
        }
        for interruption in [2, 3] {
            let root = tempfile::tempdir().unwrap();
            WriterLease::enable_directory_moves(root.path()).unwrap();
            let source = root.path().join("source");
            fs::create_dir(&source).unwrap();
            let path = source.join("project.vibecut");
            let mut writer = WriterLease::acquire(&path).unwrap();
            let project = Project::empty(crate::ProjectId(1), "recoverable".into()).unwrap();
            writer.save(&project).unwrap();
            drop(writer);
            fs::write(source.join("media"), b"complete media").unwrap();
            let destination = root.path().join(".undo-copy");
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "persistence::writer::tests::interrupted_directory_move_keeps_complete_reacquirable_project",
                ])
                .env(CHILD_ROOT, root.path())
                .env(CHILD_PHASE, interruption.to_string())
                .output()
                .unwrap();
            assert_eq!(child.status.code(), Some(77), "{child:?}");
            assert_ne!(source.exists(), destination.exists());
            let retained = if source.exists() {
                &source
            } else {
                &destination
            };
            assert_eq!(
                WriterLease::acquire(&retained.join("project.vibecut"))
                    .unwrap()
                    .load()
                    .unwrap()
                    .project,
                project
            );
            assert_eq!(fs::read(retained.join("media")).unwrap(), b"complete media");
        }
    }

    #[cfg(unix)]
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
