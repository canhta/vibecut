//! Publishes admitted output artifacts.
use super::{Error as RuntimeError, Result};
use project::atomic::{AtomicFs, Overwrite, StreamingStage, StreamingWriter};
use std::path::{Path, PathBuf};

pub(crate) struct Publication<'a> {
    pub project: &'a project::Project,
    pub path: &'a Path,
    pub protected: &'a [PathBuf],
    pub hooks: &'a dyn render::ExecutionHooks,
}

impl Publication<'_> {
    pub fn write<E: From<RuntimeError>>(
        &self,
        overwrite: Overwrite,
        max_bytes: u64,
        fs: &dyn AtomicFs,
        write: impl FnOnce(&mut StreamingWriter) -> std::result::Result<(), E>,
    ) -> std::result::Result<(), E> {
        self.admit()?;
        let stage =
            StreamingStage::new(self.path, overwrite, max_bytes).map_err(RuntimeError::from)?;
        {
            let mut writer = stage.writer().map_err(RuntimeError::from)?;
            write(&mut writer)?;
        }
        let finalized = stage.finalize_with(fs).map_err(RuntimeError::from)?;
        self.admit()?;
        self.hooks.begin_publish().map_err(RuntimeError::from)?;
        finalized.publish_with(fs).map_err(RuntimeError::from)?;
        Ok(())
    }

    /// Publish a completed frame directory under the destination's name. The frames are
    /// renamed `<destination name>_%05d.png` in order, staged beside the destination and
    /// promoted as one directory, so cancellation or failure leaves no partial result.
    pub fn write_sequence(
        &self,
        overwrite: Overwrite,
        max_bytes: u64,
        fs: &dyn AtomicFs,
        source: &Path,
    ) -> Result<()> {
        self.admit()?;
        let (mut stage, prefix) = render::sequence_stage(self.path, overwrite, max_bytes)?;
        let mut frames = Vec::new();
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                frames.push(entry.path());
            }
        }
        // Frame names are fixed-width, so name order is frame order.
        frames.sort();
        for (index, frame) in frames.iter().enumerate() {
            if self.hooks.cancelled() {
                return Err(RuntimeError::Cancelled);
            }
            let _lease = self.hooks.reserve(
                render::MemoryKind::Export,
                std::fs::metadata(frame)?.len() + (1 << 20),
            )?;
            let bytes = std::fs::read(frame)?;
            stage.write_file(
                &render::sequence_frame_name(&prefix, index as u64 + 1),
                &bytes,
            )?;
        }
        let finalized = stage.finalize_with(fs)?;
        self.admit()?;
        self.hooks.begin_publish()?;
        finalized.publish_with(fs)?;
        Ok(())
    }

    pub(crate) fn admit(&self) -> Result<()> {
        if self.hooks.cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        render::validate_output_path(self.project, self.path, self.hooks)?;
        project::atomic::admit_destination(self.path)?;
        let normalized = comparison_path(normalize(self.path)?);
        for path in self
            .protected
            .iter()
            .map(PathBuf::as_path)
            .chain(self.project.data().assets.iter().map(|asset| asset.path()))
        {
            if self.path == path
                || normalize(path).is_ok_and(|canonical| {
                    let canonical = comparison_path(canonical);
                    canonical == normalized || (path.is_dir() && normalized.starts_with(canonical))
                })
                || self.same_file(path)?
            {
                return Err(RuntimeError::Edit(crate::EditError::Invalid(
                    "output aliases an active project file",
                )));
            }
        }
        Ok(())
    }

    fn same_file(&self, path: &Path) -> Result<bool> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if let (Ok(a), Ok(b)) = (std::fs::metadata(self.path), std::fs::metadata(path))
                && a.dev() == b.dev()
                && a.ino() == b.ino()
            {
                return Ok(true);
            }
        }
        // Compare live source bytes, never the hash recorded at import: a source
        // may have changed since then. Conservatively refuse identical copies.
        Ok(self.path.is_file()
            && path.is_file()
            && media::ContentHash::read_with_cancel(
                self.path,
                || self.hooks.cancelled(),
                |_, _| {},
            )? == media::ContentHash::read_with_cancel(
                path,
                || self.hooks.cancelled(),
                |_, _| {},
            )?)
    }
}

fn normalize(path: &Path) -> Result<PathBuf> {
    Ok(std::fs::canonicalize(path).or_else(|_| {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        Ok::<_, std::io::Error>(
            std::fs::canonicalize(parent)?
                .join(path.file_name().ok_or(std::io::ErrorKind::InvalidInput)?),
        )
    })?)
}

fn comparison_path(path: PathBuf) -> PathBuf {
    // Canonicalize the existing parent first, then fold missing filenames too.
    // Lossy/case-fold collisions are conservative refusals, never admissions.
    #[cfg(windows)]
    {
        PathBuf::from(path.as_os_str().to_string_lossy().to_uppercase())
    }
    #[cfg(not(windows))]
    {
        path
    }
}

mod file_names;
pub(crate) use file_names::snapshot_project_name;
pub use file_names::{default_file_name, sanitize_file_name};
