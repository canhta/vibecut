//! Parses the trusted CLI control-session options.
use crate::args::{CliError, utf8};
use crate::output::{CliConsent, Output};
use crate::session::Context;
use serde_json::json;
use std::sync::Arc;
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
                     [--yes-upload] [--accept-unknown-price]\n\
                     Reads newline-delimited control requests on stdin and writes one JSON\n\
                     response per line. The session saves changed project/metadata on EOF\n\
                     unless --no-save is set; --allow-overwrite grants FilePublish overwrite.\n\
                     --yes-upload approves the remote requests the session's commands make\n\
                     (for example dub.generate), each shown on stderr first; without it they\n\
                     are refused. --accept-unknown-price also allows a request of unknown price.";
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
    let mut yes_upload = false;
    let mut accept_unknown_price = false;
    for arg in args {
        match utf8(&arg, "control flag")? {
            "--allow-overwrite" => allow_overwrite = true,
            "--no-save" => save = false,
            "--yes-upload" => yes_upload = true,
            "--accept-unknown-price" => accept_unknown_price = true,
            other => return Err(CliError::Usage(format!("unknown control flag: {other}"))),
        }
    }

    crate::session::control(
        context,
        project_path,
        allow_overwrite,
        save,
        Arc::new(CliConsent::new(yes_upload, accept_unknown_price)),
    )
}
fn control_usage() -> CliError {
    CliError::Usage(
        "usage: vibecut-cli control <project> [--allow-overwrite] [--no-save] [--yes-upload] \
         [--accept-unknown-price]"
            .into(),
    )
}
