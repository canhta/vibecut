//! Bounded same-directory streaming output with explicit publication.
use super::atomic::{Overwrite, parent, sync_parent};
use crate::{Error, Result};
use std::{
    fs::{self, File},
    io::{Seek, SeekFrom, Write},
    path::Path,
};

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
