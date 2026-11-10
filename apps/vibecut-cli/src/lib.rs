//! Wires the public headless CLI entry points.
mod args;
mod commands;
mod mcp;
mod output;
mod session;

pub use args::CliError;
pub use output::Output;
use std::ffi::OsString;

pub fn run(args: Vec<OsString>) -> Result<Output, CliError> {
    commands::run(&session::Context::default(), args)
}
