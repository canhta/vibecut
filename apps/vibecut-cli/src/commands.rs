//! Routes a native command to its owning adapter.
mod ai;
mod catalog;
mod color;
mod libraries;
mod media;
mod project;
use crate::args::{CliError, HELP, utf8};
use crate::output::Output;
use crate::session::Context;
use serde_json::json;
use std::ffi::OsString;

pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Err(CliError::Usage(
            "missing subcommand; run `vibecut-cli --help`".into(),
        ));
    };
    let rest: Vec<OsString> = args.collect();
    let command = utf8(&command, "subcommand")?;
    match command {
        "--help" | "-h" | "help" => Ok(Output::new(
            json!({"usage": HELP}),
            HELP.to_owned(),
            false,
            0,
        )),
        "--version" | "-V" => Ok(Output::new(
            json!({"version": env!("CARGO_PKG_VERSION")}),
            format!("vibecut-cli {}", env!("CARGO_PKG_VERSION")),
            false,
            0,
        )),
        "color" => color::run(context, rest),
        "probe" => media::probe(context, rest),
        "media" => media::generate(context, rest),
        "project" => project::project(context, rest),
        "exec" => project::exec(context, rest),
        "normalize" => media::normalize(context, rest),
        "scenes" => media::scenes(context, rest),
        "control" => project::control(context, rest),
        "mcp" => crate::mcp::run(context, rest),
        "export" => media::export(context, rest),
        "batch" => media::batch(context, rest),
        "captions" => project::captions(context, rest),
        "cut" => project::cut(context, rest),
        "transcript" => project::transcript(context, rest),
        "subtitle" => ai::subtitle(context, rest),
        "clips" => ai::clips(context, rest),
        "chapters" => ai::chapters(context, rest),
        "fillers" => ai::fillers(context, rest),
        "providers" => ai::providers(context, rest),
        "sounds" => libraries::sounds(context, rest),
        "model" => ai::model(context, rest),
        "fonts" => libraries::fonts(context, rest),
        "commands" => catalog::commands(context, rest),
        "schema" => catalog::schema(context, rest),
        other => Err(CliError::Usage(format!("unknown subcommand: {other}"))),
    }
}
