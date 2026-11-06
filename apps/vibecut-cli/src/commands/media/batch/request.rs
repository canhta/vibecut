//! Parses trusted native batch command options.
use super::{execute, load};
use crate::session::Context;
use crate::{CliError, Output};
use serde_json::json;
use std::ffi::OsString;
use std::path::PathBuf;

pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut path = None;
    let mut allow_degraded = false;
    let mut allow_overwrite = false;
    for arg in args {
        match arg.to_str() {
            Some("--allow-degraded") => allow_degraded = true,
            Some("--allow-overwrite") => allow_overwrite = true,
            Some("--help" | "-h") => {
                return Ok(Output::new(
                    json!({"usage": batch_usage_text()}),
                    batch_usage_text().to_owned(),
                    false,
                    0,
                ));
            }
            Some(other) if other.starts_with('-') => {
                return Err(CliError::Usage(format!("unknown batch flag: {other}")));
            }
            _ => {
                if path.is_some() {
                    return Err(CliError::Usage("batch takes one job-file path".into()));
                }
                path = Some(PathBuf::from(arg));
            }
        }
    }
    let path = path.ok_or_else(batch_usage)?;
    let batch = load(&path, allow_degraded, allow_overwrite)?;
    execute(context, batch, engine::submit_export)
}

fn batch_usage_text() -> &'static str {
    "usage: vibecut-cli batch <job-file> [--allow-degraded] [--allow-overwrite]"
}
fn batch_usage() -> CliError {
    CliError::Usage(batch_usage_text().into())
}
