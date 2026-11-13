//! Adapts bundle export and import to the project crate's bundle owner.
use super::lifecycle::two_paths;
use crate::args::{CliError, utf8};
use crate::output::{Output, json_path};
use crate::session::Context;
use project::{WriterLease, bundle};
use serde_json::json;
use std::ffi::OsString;

pub(crate) fn run(_context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let action = args.next().ok_or_else(|| {
        CliError::Usage("usage: vibecut-cli bundle export|import <from> <to>".into())
    })?;
    let rest: Vec<OsString> = args.collect();
    match utf8(&action, "bundle action")? {
        "export" => {
            let (project, directory) = two_paths(
                rest,
                "usage: vibecut-cli bundle export <project> <bundle-dir>",
            )?;
            export(project, directory)
        }
        "import" => {
            let (bundle, directory) = two_paths(
                rest,
                "usage: vibecut-cli bundle import <bundle-dir> <project-dir>",
            )?;
            import(bundle, directory)
        }
        other => Err(CliError::Usage(format!(
            "unknown bundle action: {other}; expected export or import"
        ))),
    }
}

/// Bundle the saved project. The writer lease keeps the bytes stable while read;
/// the destination must not exist.
fn export(project: std::path::PathBuf, directory: std::path::PathBuf) -> Result<Output, CliError> {
    let lease = WriterLease::acquire(&project)?;
    let loaded = lease.load()?;
    let exported = bundle::export(&loaded.project, &directory)?;
    let missing: Vec<u64> = exported.missing().iter().map(|id| id.0).collect();
    let human = format!(
        "bundled {} media file(s) into {}{}",
        exported.copied(),
        exported.path().display(),
        if missing.is_empty() {
            String::new()
        } else {
            format!(
                "; {} asset(s) had no media and will need relinking",
                missing.len()
            )
        }
    );
    let value = json!({
        "path": json_path(exported.path())?,
        "copied": exported.copied(),
        "missing": missing,
    });
    Ok(Output::new(value, human, false, 0))
}

/// Extract into a new project directory. Missing media is a relink state, not a failure.
fn import(
    bundle_dir: std::path::PathBuf,
    directory: std::path::PathBuf,
) -> Result<Output, CliError> {
    let imported = bundle::import(&bundle_dir, &directory)?;
    let missing: Vec<u64> = imported.missing().iter().map(|id| id.0).collect();
    let human = format!(
        "imported {}{}",
        imported.project_path().display(),
        if missing.is_empty() {
            String::new()
        } else {
            format!("; {} asset(s) need relinking", missing.len())
        }
    );
    let value = json!({
        "path": json_path(imported.project_path())?,
        "id": imported.project().data().id.0.to_string(),
        "missing": missing,
    });
    Ok(Output::new(value, human, false, 0))
}
