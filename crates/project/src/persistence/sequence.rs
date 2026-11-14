//! Bounded same-parent staging of one output directory with atomic promotion.
use super::atomic::{Overwrite, admit_destination, parent, sync_parent};
use crate::{Error, Result};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

/// Decides whether an entry of an existing destination may be replaced with it.
pub type Owns = Box<dyn Fn(&str) -> bool + Send>;

/// Private `.vibecut-export-*` directory beside the destination, removed on drop.
/// Only `FinalizedDirectory::publish` creates the destination name, so a failed or
/// cancelled run leaves no partial final directory and keeps any existing one.
pub struct DirectoryStage {
    staged: tempfile::TempDir,
    destination: PathBuf,
    overwrite: Overwrite,
    owns: Owns,
    max_bytes: u64,
    written: u64,
}
pub struct FinalizedDirectory(DirectoryStage);

impl DirectoryStage {
    /// Admits `path` as a new directory name or, with `Replace`, as a directory that
    /// holds only files `owns` accepts (or nothing). Anything else refuses before
    /// staging, so unrelated files are never discarded.
    pub fn new(path: &Path, overwrite: Overwrite, max_bytes: u64, owns: Owns) -> Result<Self> {
        if max_bytes == 0 {
            return Err(Error::Invalid("zero staging limit"));
        }
        admit(path, overwrite, &owns)?;
        Ok(Self {
            staged: tempfile::Builder::new()
                .prefix(".vibecut-export-")
                .tempdir_in(parent(path))?,
            destination: path.to_owned(),
            overwrite,
            owns,
            max_bytes,
            written: 0,
        })
    }
    /// Writes one new file under a single-component `name`, synced before return.
    pub fn write_file(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        component(name)?;
        let total = self
            .written
            .checked_add(bytes.len() as u64)
            .filter(|total| *total <= self.max_bytes)
            .ok_or(Error::TooLarge("directory stage"))?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.staged.path().join(name))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        self.written = total;
        Ok(())
    }
    pub fn finalize(self) -> Result<FinalizedDirectory> {
        if let Ok(directory) = fs::File::open(self.staged.path()) {
            let _ = directory.sync_all();
        }
        Ok(FinalizedDirectory(self))
    }
}

impl FinalizedDirectory {
    /// Promotes the staged directory. An existing destination is moved aside first
    /// and restored if the promotion fails, so it is never half replaced.
    pub fn publish(self) -> Result<()> {
        let Self(stage) = self;
        let existed = admit(&stage.destination, stage.overwrite, &stage.owns)?;
        let staged = stage.staged.path().to_owned();
        let aside = if existed {
            let reserved = tempfile::Builder::new()
                .prefix(".vibecut-export-old-")
                .tempdir_in(parent(&stage.destination))?;
            let path = reserved.path().to_owned();
            drop(reserved);
            fs::rename(&stage.destination, &path)?;
            Some(path)
        } else {
            None
        };
        if let Err(error) = fs::rename(&staged, &stage.destination) {
            if let Some(aside) = &aside {
                let _ = fs::rename(aside, &stage.destination);
            }
            return Err(Error::Io(error));
        }
        sync_parent(&stage.destination);
        if let Some(aside) = aside {
            let _ = fs::remove_dir_all(aside);
        }
        Ok(())
    }
}

/// Whether the destination exists. Refuses symlinks, files, a clobber under
/// `Refuse` and any existing entry that is not one of the owner's files.
fn admit(path: &Path, overwrite: Overwrite, owns: &Owns) -> Result<bool> {
    admit_destination(path)?;
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
            Err(Error::Invalid("destination is not a directory"))
        }
        Ok(_) if overwrite == Overwrite::Refuse => Err(Error::Io(std::io::Error::from(
            std::io::ErrorKind::AlreadyExists,
        ))),
        Ok(_) => {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let regular = entry.file_type()?.is_file();
                if !regular || !entry.file_name().to_str().is_some_and(|name| owns(name)) {
                    return Err(Error::Invalid("destination holds other files"));
                }
            }
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// One plain file name: no separators, drive or stream markers, dots or controls.
fn component(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 255
        || name.starts_with('.')
        || name.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
    {
        return Err(Error::Invalid("staged file name"));
    }
    Ok(())
}
