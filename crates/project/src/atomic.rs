//! Shared same-directory staging for project, export and preference writers.
use crate::{Error, Result};
use std::{
    fs::{self, File},
    io::Write,
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
/// A destination symlink is refused. Callers serialize writes to a given target.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    atomic_write_with(path, |file| file.write_all(bytes))
}
fn atomic_write_with(
    path: &Path,
    write: impl FnOnce(&mut File) -> std::io::Result<()>,
) -> Result<()> {
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
    staged.persist(path).map_err(|e| Error::Io(e.error))?;
    sync_parent(path);
    Ok(())
}
pub(crate) fn atomic_write_new(path: &Path, bytes: &[u8]) -> Result<()> {
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
fn sync_parent(path: &Path) {
    if let Ok(dir) = File::open(parent(path)) {
        let _ = dir.sync_all();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_staging_preserves_original_and_removes_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.vibecut");
        fs::write(&path, b"old").unwrap();
        assert!(
            atomic_write_with(&path, |file| {
                file.write_all(b"partial")?;
                Err(std::io::Error::other("full disk"))
            })
            .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
