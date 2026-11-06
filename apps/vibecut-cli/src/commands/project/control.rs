//! Parses the trusted CLI control-session options.
use crate::args::{CliError, utf8};
use crate::output::Output;
use crate::session::Context;
use serde_json::json;
use std::{ffi::OsString, path::PathBuf};
pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let Some(project) = args.next() else {
        return Err(control_usage());
    };
    if project
        .to_str()
        .is_some_and(|value| value == "--help" || value == "-h")
    {
        let usage = "usage: vibecut-cli control <project> [--allow-overwrite] [--no-save]\n\
                     Reads newline-delimited control requests on stdin and writes one JSON\n\
                     response per line. The session saves changed project/metadata on EOF\n\
                     unless --no-save is set; --allow-overwrite grants FilePublish overwrite.";
        return Ok(Output::new(
            json!({"usage": usage}),
            usage.to_owned(),
            false,
            0,
        ));
    }
    let project_path = PathBuf::from(project);
    let mut allow_overwrite = false;
    let mut save = true;
    for arg in args {
        match utf8(&arg, "control flag")? {
            "--allow-overwrite" => allow_overwrite = true,
            "--no-save" => save = false,
            other => return Err(CliError::Usage(format!("unknown control flag: {other}"))),
        }
    }

    crate::session::control(context, project_path, allow_overwrite, save)
}
fn control_usage() -> CliError {
    CliError::Usage("usage: vibecut-cli control <project> [--allow-overwrite] [--no-save]".into())
}
