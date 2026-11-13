//! Parses CLI export requests for session admission.
use crate::args::{CliError, utf8};
use crate::output::Output;
use crate::session::Aspect;
use crate::session::{Context, export_project};
use std::{ffi::OsString, path::PathBuf};
pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let project_path = args.next().map(PathBuf::from).ok_or_else(export_usage)?;
    let mut output = None;
    let mut aspect = None;
    let mut allow_degraded = false;
    let mut overwrite = false;
    let mut options = render::OutputOptions::default();
    while let Some(arg) = args.next() {
        match utf8(&arg, "export flag")? {
            "--settings" => {
                let value = args.next().ok_or_else(export_usage)?;
                options = serde_json::from_str(utf8(&value, "export settings")?)
                    .map_err(|_| CliError::Usage("invalid typed export settings".into()))?;
            }
            "--out" => {
                output = Some(PathBuf::from(args.next().ok_or_else(|| {
                    CliError::Usage("--out requires an output path".into())
                })?));
            }
            "--aspect" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Usage("--aspect requires 9:16 or 16:9".into()))?;
                aspect = Some(match utf8(&value, "aspect")? {
                    "9:16" => Aspect::Portrait,
                    "16:9" => Aspect::Landscape,
                    _ => {
                        return Err(CliError::Usage("--aspect must be 9:16 or 16:9".into()));
                    }
                });
            }
            "--allow-degraded" => allow_degraded = true,
            "--overwrite" => overwrite = true,
            other => {
                return Err(CliError::Usage(format!("unknown export flag: {other}")));
            }
        }
    }
    let output = output.ok_or_else(export_usage)?;
    export_project(
        context,
        project_path,
        output,
        aspect,
        allow_degraded,
        overwrite,
        options,
    )
}

fn export_usage() -> CliError {
    CliError::Usage(
        "usage: vibecut-cli export <project> --out <mp4|mov|webm|gif> [--settings <JSON>] [--aspect 9:16] [--allow-degraded] [--overwrite]"
            .into(),
    )
}
