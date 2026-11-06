//! Prints the generated engine control schema.
use crate::args::CliError;
use crate::output::Output;
use crate::session::Context;
use engine::control;
use serde_json::json;
use std::ffi::OsString;

pub(crate) fn run(_context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    if !args.is_empty() {
        return Err(CliError::Usage("usage: vibecut-cli schema".into()));
    }
    let value = json!({
        "api_version": control::API_VERSION,
        "registry_hash": control::registry_hash(),
        "contract_schema": control::contract_schema(),
    });
    let human = format!(
        "API version: {}\nRegistry digest: {}\nRequest, response and event JSON Schema 2020-12 envelopes.",
        control::API_VERSION,
        control::registry_hash()
    );
    Ok(Output::new(value, human, false, 0))
}
