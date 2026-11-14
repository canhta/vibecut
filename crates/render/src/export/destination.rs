//! Refuses export destinations that alias project source media.

use crate::{Error, ExecutionHooks, Result, check};
use project::Project;
use std::path::Path;

/// Refuse destination aliases of source media before staging any exported file.
pub fn validate_output_path(
    project: &Project,
    path: &Path,
    hooks: &dyn ExecutionHooks,
) -> Result<()> {
    check(hooks)?;
    // Refuse direct/symlink/hard-link aliases of every project source. Missing
    // destinations are normalized through their existing parent before staging.
    let normalized = if path.exists() {
        std::fs::canonicalize(path)?
    } else {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::canonicalize(parent)?
            .join(path.file_name().ok_or(Error::Invalid("output filename"))?)
    };
    // An existing directory (an image-sequence destination) aliases every source it
    // holds: replacing it would discard them. It has no bytes to compare.
    if path.is_dir() {
        for asset in &project.data().assets {
            let source =
                std::fs::canonicalize(asset.path()).unwrap_or_else(|_| asset.path().to_path_buf());
            if source.starts_with(&normalized) {
                return Err(Error::Invalid("output aliases a source"));
            }
        }
        return Ok(());
    }
    // A destination that names a source is a definite alias even when another
    // source happens to hold identical bytes. Decide path identity for every
    // asset before the content fallback below can classify the destination as
    // a possible alias.
    for asset in &project.data().assets {
        let source = std::fs::canonicalize(asset.path()).or_else(|_| {
            let parent = asset.path().parent().unwrap_or(Path::new("."));
            std::fs::canonicalize(parent)
                .map(|parent| parent.join(asset.path().file_name().unwrap_or_default()))
        });
        if path == asset.path()
            || normalized == asset.path()
            || source.is_ok_and(|source| normalized == source)
        {
            return Err(Error::Invalid("output aliases a source"));
        }
    }
    // Where canonical paths do not prove identity, refuse a destination that
    // shares a source's file identity or, on platforms without file identity,
    // its complete content. These checks stay conservative: an overwrite that
    // may touch source bytes is refused before any staging.
    for asset in &project.data().assets {
        #[cfg(unix)]
        if let (Ok(output), Ok(source)) = (std::fs::metadata(path), std::fs::metadata(asset.path()))
        {
            use std::os::unix::fs::MetadataExt;
            if output.dev() == source.dev() && output.ino() == source.ino() {
                return Err(Error::Invalid("output aliases a source by hard link"));
            }
        }
        #[cfg(not(unix))]
        if path.exists()
            && media::ContentHash::read_with_cancel(path, || hooks.cancelled(), |_, _| {})?
                == asset.content_hash()
        {
            return Err(Error::Invalid("output may alias source content"));
        }
    }
    Ok(())
}
