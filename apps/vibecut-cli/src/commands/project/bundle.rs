//! Routes bundle export and import through the engine registry.
use super::lifecycle::{published, run_on_project, two_paths};
use crate::args::{CliError, utf8};
use crate::output::{Output, json_path};
use crate::session::{Context, Lent, cli_runner, run_command, standalone_session};
use serde_json::json;
use std::ffi::OsString;

pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let action = args.next().ok_or_else(|| {
        CliError::Usage("usage: vibecut-cli bundle export|import <from> <to>".into())
    })?;
    let rest: Vec<OsString> = args.collect();
    match utf8(&action, "bundle action")? {
        "export" => {
            let (project_file, directory) = two_paths(
                rest,
                "usage: vibecut-cli bundle export <project> <bundle-dir>",
            )?;
            let params = json!({"path": json_path(&directory)?});
            let result = run_on_project(context, &project_file, "project.export_bundle", params)?;
            published(result, "bundled")
        }
        "import" => {
            let (bundle_dir, directory) = two_paths(
                rest,
                "usage: vibecut-cli bundle import <bundle-dir> <project-dir>",
            )?;
            let params = json!({
                "bundle": json_path(&bundle_dir)?,
                "destination": json_path(&directory)?,
            });
            let mut session = standalone_session()?;
            let mut runner = cli_runner(context.budget()?)?;
            let result = run_command(
                &mut session,
                &mut runner,
                Lent::default(),
                "project.import_bundle",
                params,
                true,
            );
            runner.shutdown();
            published(result?, "imported")
        }
        other => Err(CliError::Usage(format!(
            "unknown bundle action: {other}; expected export or import"
        ))),
    }
}
