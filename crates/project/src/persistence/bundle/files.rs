//! Bounded, symlink-refusing file operations both bundle halves share.
use crate::persistence::atomic;
use crate::{Error, Result};
use media::ContentHash;
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use super::manifest::MAX_MEDIA_BYTES;

/// A private staging directory beside the destination, published by one rename.
pub(super) struct Target {
    pub staging: tempfile::TempDir,
    pub destination: PathBuf,
}

/// Admit a not-yet-existing destination directory and stage next to it. The
/// destination is absolute: its parent is canonical and its name is native.
pub(super) fn target(destination: &Path) -> Result<Target> {
    atomic::admit_destination(destination)?;
    let name: OsString = destination
        .file_name()
        .ok_or(Error::Invalid("bundle destination"))?
        .to_owned();
    let parent = fs::canonicalize(atomic::parent(destination))?;
    if !fs::metadata(&parent)?.is_dir() {
        return Err(Error::Invalid("bundle destination parent"));
    }
    let destination = parent.join(name);
    absent(&destination)?;
    let staging = tempfile::Builder::new()
        .prefix(".vibecut-bundle-")
        .tempdir_in(&parent)?;
    Ok(Target {
        staging,
        destination,
    })
}

fn absent(destination: &Path) -> Result<()> {
    match fs::symlink_metadata(destination) {
        Ok(_) => Err(Error::Invalid("bundle destination exists")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

impl Target {
    /// Rename the complete staging directory into place. The destination must
    /// still be absent; a failed rename removes the staging directory.
    pub fn publish(self) -> Result<PathBuf> {
        absent(&self.destination)?;
        let staged = self.staging.keep();
        if let Err(error) = fs::rename(&staged, &self.destination) {
            let _ = fs::remove_dir_all(&staged);
            return Err(error.into());
        }
        atomic::sync_parent(&self.destination);
        Ok(self.destination)
    }
}

/// Create a new file with synced bytes; never replaces.
pub(super) fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

/// Whether `path` is a directory that is not a link; false when absent.
pub(super) fn plain_directory(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_dir() => Ok(true),
        Ok(_) => Err(Error::Invalid("bundle directory is not a plain directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// Open a regular file without following a link; `None` when absent. Returns the
/// handle with its length.
pub(super) fn open_regular(path: &Path) -> Result<Option<(File, u64)>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_file() => {}
        Ok(_) => return Err(Error::Invalid("bundle entry is not a regular file")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let file = File::open(path)?;
    let length = file.metadata()?.len();
    Ok(Some((file, length)))
}

/// Read a required regular file, refusing more than `limit` bytes.
pub(super) fn read_required(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let (file, _) = open_regular(path)?.ok_or(Error::Invalid("bundle file missing"))?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(Error::TooLarge("bundle file"));
    }
    Ok(bytes)
}

/// Copy at most [`MAX_MEDIA_BYTES`] into a new synced file, then hash the copy
/// itself so a source that changed while copying is caught. Returns the length.
pub(super) fn copy_verified(input: File, target: &Path, expected: ContentHash) -> Result<u64> {
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)?;
    let copied = std::io::copy(&mut input.take(MAX_MEDIA_BYTES + 1), &mut output)?;
    if copied > MAX_MEDIA_BYTES {
        return Err(Error::TooLarge("bundle media"));
    }
    output.sync_all()?;
    drop(output);
    if ContentHash::read(target)? != expected {
        return Err(Error::Invalid(
            "media content differs from its recorded hash",
        ));
    }
    Ok(copied)
}
