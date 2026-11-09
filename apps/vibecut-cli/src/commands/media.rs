//! Groups native media command adapters.
mod probe;
pub(super) use probe::run as probe;
mod scenes;
pub(super) use scenes::run as scenes;
mod normalize;
pub(super) use normalize::run as normalize;
mod export;
pub(super) use export::run as export;
mod batch;
pub(super) use batch::run as batch;
mod thumbnails;
pub(super) use thumbnails::run as thumbnails;
mod waveform;
pub(super) use waveform::run as waveform;

use crate::args::{CliError, utf8};
use crate::output::Output;
use crate::session::Context;
use serde_json::json;
use std::ffi::OsString;

/// Routes `media waveform|thumbnails`.
pub(crate) fn generate(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Err(CliError::Usage(
            "missing media subcommand; run `vibecut-cli --help`".into(),
        ));
    };
    let rest: Vec<OsString> = args.collect();
    match utf8(&command, "media subcommand")? {
        "waveform" => waveform(context, rest),
        "thumbnails" => thumbnails(context, rest),
        "--help" | "-h" | "help" => Ok(Output::new(
            json!({"usage": help_text()}),
            help_text().to_owned(),
            false,
            0,
        )),
        other => Err(CliError::Usage(format!(
            "unknown media subcommand: {other}"
        ))),
    }
}

fn help_text() -> &'static str {
    "usage: vibecut-cli media waveform <project> --asset <id>\n       \
     vibecut-cli media thumbnails <project> --item <id> --pixels-per-second <zoom> \
     [--start <tick> --end <tick>]"
}
