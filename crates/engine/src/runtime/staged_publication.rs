//! Publishes admitted output artifacts.
use crate::{EditError, Result};
use project::atomic::{AtomicFs, Overwrite, StreamingStage, StreamingWriter};
use std::path::{Path, PathBuf};

pub(crate) struct Publication<'a> {
    pub project: &'a project::Project,
    pub path: &'a Path,
    pub protected: &'a [PathBuf],
    pub hooks: &'a dyn render::ExecutionHooks,
}

impl Publication<'_> {
    pub fn write(
        &self,
        overwrite: Overwrite,
        max_bytes: u64,
        fs: &dyn AtomicFs,
        write: impl FnOnce(&mut StreamingWriter) -> Result<()>,
    ) -> Result<()> {
        self.admit()?;
        let stage = StreamingStage::new(self.path, overwrite, max_bytes)?;
        {
            let mut writer = stage.writer()?;
            write(&mut writer)?;
        }
        let finalized = stage.finalize_with(fs)?;
        self.admit()?;
        self.hooks.begin_publish()?;
        finalized.publish_with(fs)?;
        Ok(())
    }

    fn admit(&self) -> Result<()> {
        if self.hooks.cancelled() {
            return Err(EditError::Cancelled);
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
                return Err(EditError::Invalid("output aliases an active project file"));
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
            && media::ContentHash::read_with_cancel(self.path, || self.hooks.cancelled())?
                == media::ContentHash::read_with_cancel(path, || self.hooks.cancelled())?)
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
