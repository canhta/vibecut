//! Prints the generated engine command catalog.
use crate::args::CliError;
use crate::output::Output;
use crate::session::Context;
use engine::control;
use serde_json::json;
use std::ffi::OsString;

pub(crate) fn run(_context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    if !args.is_empty() {
        return Err(CliError::Usage("usage: vibecut-cli commands".into()));
    }
    let registry = engine::registry();
    let value = json!({
        "api_version": control::API_VERSION,
        "registry_hash": control::registry_hash(),
        "limits": control::LIMITS,
        "output_schema": control::output_schema(),
        "entries": registry,
    });
    let mut human = format!(
        "API version: {}\nRegistry digest: {}\n",
        control::API_VERSION,
        control::registry_hash()
    );
    human.push_str(
        &registry
            .iter()
            .map(|spec| {
                let fields = spec
                    .params_schema
                    .get("properties")
                    .and_then(serde_json::Value::as_object)
                    .map(|fields| fields.keys().cloned().collect::<Vec<_>>().join(", "))
                    .unwrap_or_default();
                if fields.is_empty() {
                    format!("{} - {}", spec.id, spec.label)
                } else {
                    format!("{} - {} ({fields})", spec.id, spec.label)
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
    );
    Ok(Output::new(value, human, false, 0))
}
