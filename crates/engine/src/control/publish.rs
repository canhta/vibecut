//! Publishes the open project as a copy or bundle, and extracts a bundle.
use super::{Error as EngineError, Result};
use crate::session::wire::ControlResult;
use project::{Metadata, Project, WriterLease, bundle};
use std::path::Path;

/// Write a copy of `project` under a fresh identity, with its view metadata. An
/// existing destination is refused unless `replace` is set, in which case the
/// writer lease publishes over it atomically. The open project's own files
/// (`protected`, including aliases) are never a destination. The open project and
/// its lease are untouched.
pub(super) fn save_copy(
    project: &Project,
    metadata: &Metadata,
    path: &Path,
    replace: bool,
    protected: &[std::path::PathBuf],
) -> Result<ControlResult> {
    let canonical = std::fs::canonicalize(path).ok();
    if protected.iter().any(|own| {
        own == path
            || canonical
                .as_ref()
                .is_some_and(|target| std::fs::canonicalize(own).ok().as_ref() == Some(target))
    }) {
        return Err(EngineError::Edit(crate::EditError::Invalid(
            "save destination is the open project",
        )));
    }
    match std::fs::symlink_metadata(path) {
        Ok(_) if replace => {}
        Ok(_) => {
            return Err(EngineError::Edit(crate::EditError::Invalid(
                "save destination exists",
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(project::Error::from(error).into()),
    }
    let mut lease = WriterLease::acquire(path)?;
    let receipt = lease.save(&project.fork()?)?;
    lease.save_metadata(metadata)?;
    Ok(ControlResult::Published {
        path: receipt.path().to_owned(),
        project_id: receipt.project_id().0.to_string(),
        copied: 0,
        missing: Vec::new(),
        missing_luts: Vec::new(),
    })
}

/// Write `project`, its media and its imported LUTs as a new bundle directory.
pub(super) fn export_bundle(project: &Project, path: &Path) -> Result<ControlResult> {
    let exported = bundle::export(project, path)?;
    Ok(ControlResult::Published {
        path: exported.path().to_owned(),
        project_id: project.data().id.0.to_string(),
        copied: exported.copied(),
        missing: exported.missing().iter().map(|id| id.0).collect(),
        missing_luts: exported.missing_luts().iter().map(|id| id.hex()).collect(),
    })
}

/// Extract a bundle into the new directory `destination` under a fresh identity.
/// `copied` counts the assets whose media came with the bundle.
pub(super) fn import_bundle(bundle_dir: &Path, destination: &Path) -> Result<ControlResult> {
    let imported = bundle::import(bundle_dir, destination)?;
    let data = imported.project().data();
    Ok(ControlResult::Published {
        path: imported.project_path().to_owned(),
        project_id: data.id.0.to_string(),
        copied: data.assets.len().saturating_sub(imported.missing().len()),
        missing: imported.missing().iter().map(|id| id.0).collect(),
        missing_luts: imported.missing_luts().iter().map(|id| id.hex()).collect(),
    })
}
