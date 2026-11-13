//! Publishes the open project as a copy or bundle, and extracts a bundle.
use super::{Error as EngineError, Result};
use crate::session::wire::ControlResult;
use project::{Metadata, Project, WriterLease, bundle};
use std::path::Path;

/// Write a copy of `project` under a fresh identity at a path that does not yet
/// exist, with its view metadata. The open project and its lease are untouched.
pub(super) fn save_copy(
    project: &Project,
    metadata: &Metadata,
    path: &Path,
) -> Result<ControlResult> {
    match std::fs::symlink_metadata(path) {
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
