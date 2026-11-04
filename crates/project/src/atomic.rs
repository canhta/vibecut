//! Shared same-directory staging for project, export and preference writers.
use crate::{Error, Result};
use std::{
    fs::{self, File},
    io::{Seek, SeekFrom, Write},
    path::Path,
};

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}
/// Replace a file using a privately created, synced same-directory staging file.
/// Failed staging/publication preserves the destination. Parent sync after commit
/// is best effort; ownership, ACLs, extended attributes and hard links are not kept.
/// Destination symlinks and the reserved writer-control namespace are refused.
/// Callers serialize writes to a given target.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    atomic_write_checked(path, bytes, || Ok(()))
}
pub(crate) fn atomic_write_checked(
    path: &Path,
    bytes: &[u8],
    before_publish: impl FnOnce() -> Result<()>,
) -> Result<()> {
    atomic_write_with(path, |file| file.write_all(bytes), before_publish)
}
fn atomic_write_with(
    path: &Path,
    write: impl FnOnce(&mut File) -> std::io::Result<()>,
    before_publish: impl FnOnce() -> Result<()>,
) -> Result<()> {
    admit_destination(path)?;
    let permissions = match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(Error::Invalid("save destination is a symlink"));
        }
        Ok(meta) => Some(meta.permissions()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let mut staged = tempfile::Builder::new()
        .prefix(".vibecut-")
        .tempfile_in(parent(path))?;
    write(staged.as_file_mut())?;
    if let Some(permissions) = permissions {
        staged.as_file().set_permissions(permissions)?;
    }
    staged.as_file().sync_all()?;
    before_publish()?;
    staged.persist(path).map_err(|e| Error::Io(e.error))?;
    sync_parent(path);
    Ok(())
}
pub(crate) fn atomic_write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    admit_destination(path)?;
    let mut staged = tempfile::Builder::new()
        .prefix(".vibecut-")
        .tempfile_in(parent(path))?;
    staged.write_all(bytes)?;
    staged.as_file().sync_all()?;
    staged
        .persist_noclobber(path)
        .map_err(|e| Error::Io(e.error))?;
    sync_parent(path);
    Ok(())
}
// Data publication and permanent lock control files have disjoint namespaces.
// Inspect native ASCII markers without converting/normalizing the filename.
pub(crate) fn admit_destination(path: &Path) -> Result<()> {
    fn name(path: &Path) -> Result<()> {
        let bytes = path
            .file_name()
            .ok_or(Error::Invalid("publication filename"))?
            .as_encoded_bytes();
        #[cfg(windows)]
        admit_windows_name(bytes)?;
        if bytes.starts_with(b".")
            && bytes
                .get(bytes.len().saturating_sub(5)..)
                .is_some_and(|suffix| suffix.eq_ignore_ascii_case(b".lock"))
        {
            return Err(Error::Unsupported(
                "destination uses the writer-lock namespace",
            ));
        }
        Ok(())
    }
    name(path)?;
    match fs::canonicalize(path) {
        Ok(canonical) => name(&canonical),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
// Win32 normalized spellings and alternate streams must not acquire distinct
// sidecars for the same destination, even before that destination exists.
#[cfg(any(windows, test))]
fn admit_windows_name(bytes: &[u8]) -> Result<()> {
    if bytes.contains(&b':') || matches!(bytes.last(), Some(b'.' | b' ')) {
        return Err(Error::Unsupported("ambiguous Windows publication filename"));
    }
    Ok(())
}
fn sync_parent(path: &Path) {
    if let Ok(dir) = File::open(parent(path)) {
        let _ = dir.sync_all();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_name_admission_refuses_normalized_and_stream_aliases() {
        for name in [
            b".target.lock.".as_slice(),
            b".target.lock ",
            b".target.lock::$DATA",
            b"project:stream",
        ] {
            assert!(matches!(
                admit_windows_name(name),
                Err(Error::Unsupported(_))
            ));
        }
        assert!(admit_windows_name(b"project.vibecut").is_ok());
        assert!(admit_windows_name(b".draft.vibecut").is_ok());
    }
    #[test]
    fn failed_staging_preserves_original_and_removes_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.vibecut");
        fs::write(&path, b"old").unwrap();
        assert!(
            atomic_write_with(
                &path,
                |file| {
                    file.write_all(b"partial")?;
                    Err(std::io::Error::other("full disk"))
                },
                || Ok(())
            )
            .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn failed_publication_precondition_preserves_destination_and_cleans_staging() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.vibecut");
        fs::write(&path, b"old").unwrap();
        assert!(matches!(
            atomic_write_checked(&path, b"new", || Err(Error::Conflict)),
            Err(Error::Conflict)
        ));
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

/// Explicit permission to replace an existing destination. No-clobber publication
/// is atomic even if another writer creates the destination after staging.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overwrite {
    Refuse,
    Replace,
}
/// Failure-injection seam. Production callers use Fs; no lock spans these calls.
pub trait AtomicFs {
    fn sync(&self, file: &File) -> std::io::Result<()>;
    fn before_publish(&self) -> std::io::Result<()> {
        Ok(())
    }
}
pub struct Fs;
impl AtomicFs for Fs {
    fn sync(&self, file: &File) -> std::io::Result<()> {
        file.sync_all()
    }
}
/// Private same-directory streaming output, removed on drop. A process crash can
/// leave a .vibecut-export-* orphan; consumers may offer explicit orphan cleanup,
/// never promote or delete unknown staging files automatically.
pub struct StreamingStage {
    staged: tempfile::NamedTempFile,
    destination: std::path::PathBuf,
    overwrite: Overwrite,
    max_bytes: u64,
    writer_active: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
pub struct FinalizedStage(StreamingStage);
impl StreamingStage {
    pub fn new(path: &Path, overwrite: Overwrite, max_bytes: u64) -> Result<Self> {
        if max_bytes == 0 {
            return Err(Error::Invalid("zero staging limit"));
        }
        destination(path, overwrite)?;
        Ok(Self {
            staged: tempfile::Builder::new()
                .prefix(".vibecut-export-")
                .tempfile_in(parent(path))?,
            destination: path.to_owned(),
            overwrite,
            max_bytes,
            writer_active: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }
    /// Pass this owned clone to media's bounded mux sink. Close it before finalize.
    pub fn writer(&self) -> Result<StreamingWriter> {
        let file = self.staged.as_file().try_clone()?;
        if self
            .writer_active
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .is_err()
        {
            return Err(Error::Invalid("streaming writer already active"));
        }
        Ok(StreamingWriter {
            file,
            limit: self.max_bytes,
            active: self.writer_active.clone(),
        })
    }
    pub fn finalize(self) -> Result<FinalizedStage> {
        self.finalize_with(&Fs)
    }
    pub fn finalize_with(self, fs: &dyn AtomicFs) -> Result<FinalizedStage> {
        if self
            .writer_active
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(Error::Invalid("close streaming writer before finalize"));
        }
        if self.staged.as_file().metadata()?.len() > self.max_bytes {
            return Err(Error::TooLarge("streaming stage"));
        }
        if let Some(permissions) = destination(&self.destination, self.overwrite)? {
            self.staged.as_file().set_permissions(permissions)?;
        }
        fs.sync(self.staged.as_file())?;
        Ok(FinalizedStage(self))
    }
}
fn destination(path: &Path, overwrite: Overwrite) -> Result<Option<std::fs::Permissions>> {
    match fs::symlink_metadata(path) {
        Ok(m) if !m.is_file() || m.file_type().is_symlink() => {
            Err(Error::Invalid("destination is not a regular file"))
        }
        Ok(_) if overwrite == Overwrite::Refuse => Err(Error::Io(std::io::Error::from(
            std::io::ErrorKind::AlreadyExists,
        ))),
        Ok(m) => Ok(Some(m.permissions())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
impl FinalizedStage {
    pub fn publish(self) -> Result<()> {
        self.publish_with(&Fs)
    }
    pub fn publish_with(self, fs: &dyn AtomicFs) -> Result<()> {
        destination(&self.0.destination, self.0.overwrite)?;
        fs.before_publish()?;
        let Self(stage) = self;
        match stage.overwrite {
            Overwrite::Replace => stage.staged.persist(&stage.destination),
            Overwrite::Refuse => stage.staged.persist_noclobber(&stage.destination),
        }
        .map_err(|e| Error::Io(e.error))?;
        sync_parent(&stage.destination);
        Ok(())
    }
}

/// Bounded seekable sink accepted directly by media's Muxer. Every write/seek
/// checks the cap before changing the private staged file.
pub struct StreamingWriter {
    file: File,
    limit: u64,
    active: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Drop for StreamingWriter {
    fn drop(&mut self) {
        self.active
            .store(false, std::sync::atomic::Ordering::Release);
    }
}
impl media::MuxSink for StreamingWriter {
    fn sync_file(&self) -> std::io::Result<File> {
        self.file.try_clone()
    }
}
impl Write for StreamingWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let end = self
            .file
            .stream_position()?
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| std::io::Error::other("staging byte limit"))?;
        if end > self.limit {
            return Err(std::io::Error::other("staging byte limit"));
        }
        self.file.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}
impl Seek for StreamingWriter {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let end = match pos {
            SeekFrom::Start(p) => i128::from(p),
            SeekFrom::End(p) => i128::from(self.file.metadata()?.len()) + i128::from(p),
            SeekFrom::Current(p) => i128::from(self.file.stream_position()?) + i128::from(p),
        };
        if end < 0 || end > i128::from(self.limit) {
            return Err(std::io::Error::other("staging seek limit"));
        }
        self.file.seek(pos)
    }
}
