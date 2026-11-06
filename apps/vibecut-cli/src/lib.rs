//! Headless CLI adapter: arguments, dispatch through engine, typed exit codes.
mod arguments;
mod batch;
mod commands;
mod consent;
mod control_session;
mod exit_codes;
mod job_runtime;
pub mod mcp;
mod output;
mod paths;
mod progress;

use self::commands::captions::captions;
use self::commands::catalog::{commands, schema};
use self::commands::clips::clips;
use self::commands::exec::exec;
use self::commands::export::export;
use self::commands::fonts::fonts;
use self::commands::model::model;
use self::commands::normalize::normalize;
use self::commands::probe::probe;
use self::commands::project::project;
use self::commands::providers::providers;
use self::commands::scenes::scenes;
use self::commands::sounds::sounds;
use self::commands::subtitle::subtitle;
use arguments::{HELP, utf8};
use control_session::control;
pub use exit_codes::CliError;
pub use output::Output;
use serde_json::json;
use std::ffi::OsString;

pub fn run(args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Err(CliError::Usage(
            "missing subcommand; run `vibecut-cli --help`".into(),
        ));
    };
    let rest: Vec<OsString> = args.collect();
    let command = utf8(&command, "subcommand")?;
    match command {
        "--help" | "-h" | "help" => Ok(Output {
            silent: false,
            exit: 0,
            value: json!({"usage": HELP}),
            human: HELP.to_owned(),
        }),
        "--version" | "-V" => Ok(Output {
            silent: false,
            exit: 0,
            value: json!({"version": env!("CARGO_PKG_VERSION")}),
            human: format!("vibecut-cli {}", env!("CARGO_PKG_VERSION")),
        }),
        "probe" => probe(rest),
        "project" => project(rest),
        "exec" => exec(rest),
        "normalize" => normalize(rest),
        "scenes" => scenes(rest),
        "control" => control(rest),
        "mcp" => mcp::run(rest),
        "export" => export(rest),
        "batch" => batch::run(rest),
        "captions" => captions(rest),
        "subtitle" => subtitle(rest),
        "clips" => clips(rest),
        "providers" => providers(rest),
        "sounds" => sounds(rest),
        "model" => model(rest),
        "fonts" => fonts(rest),
        "commands" => commands(rest),
        "schema" => schema(rest),
        other => Err(CliError::Usage(format!("unknown subcommand: {other}"))),
    }
}

#[cfg(test)]
mod export_tests;
