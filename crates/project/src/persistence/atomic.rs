//! Shared same-directory staging for project, export and preference writers.
use crate::{Error, Result};
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
};

pub use super::streaming::{AtomicFs, FinalizedStage, Fs, StreamingStage, StreamingWriter};

pub(super) fn parent(path: &Path) -> &Path {
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
    stage_validated(
        path,
        |file| file.write_all(bytes).map_err(Error::from),
        |_| Ok(()),
    )
}
/// Stage a file, let the caller validate the staged bytes, then publish.
///
/// The validator sees the complete, synced staging file at its sidecar path before
/// it replaces the destination. Refusing there preserves the prior destination and
/// removes staging. Destination admission, previous permissions, parent sync and
/// the publish guarantees are [`atomic_write`]'s.
pub fn stage_validated<T, E>(
    destination: &Path,
    write: impl FnOnce(&mut File) -> std::result::Result<(), E>,
    validate: impl FnOnce(&Path) -> std::result::Result<T, E>,
) -> std::result::Result<T, E>
where
    E: From<Error> + From<std::io::Error>,
{
    admit_destination(destination).map_err(E::from)?;
    let permissions = match fs::symlink_metadata(destination) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(E::from(Error::Invalid("save destination is a symlink")));
        }
        Ok(meta) => Some(meta.permissions()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(E::from(e)),
    };
    let mut staged = tempfile::Builder::new()
        .prefix(".vibecut-")
        .tempfile_in(parent(destination))
        .map_err(E::from)?;
    write(staged.as_file_mut())?;
    if let Some(permissions) = permissions {
        staged
            .as_file()
            .set_permissions(permissions)
            .map_err(E::from)?;
    }
    staged.as_file().sync_all().map_err(E::from)?;
    let value = validate(staged.path())?;
    staged.persist(destination).map_err(|e| E::from(e.error))?;
    sync_parent(destination);
    Ok(value)
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
/// Refuse reserved writer-control names and native path aliases before publication.
pub fn admit_destination(path: &Path) -> Result<()> {
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
pub(super) fn sync_parent(path: &Path) {
    if let Ok(dir) = File::open(parent(path)) {
        let _ = dir.sync_all();
    }
}

/// Explicit permission to replace an existing destination. No-clobber publication
/// is atomic even if another writer creates the destination after staging.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overwrite {
    Refuse,
    Replace,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
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
            stage_validated(
                &path,
                |file| {
                    file.write_all(b"partial")?;
                    Err(Error::from(std::io::Error::other("full disk")))
                },
                |_| Ok(()),
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
            stage_validated(
                &path,
                |file| file.write_all(b"new").map_err(Error::from),
                |staged| {
                    assert_eq!(fs::read(staged).unwrap(), b"new");
                    Err::<(), Error>(Error::Conflict)
                }
            ),
            Err(Error::Conflict)
        ));
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
