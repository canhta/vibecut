use crate::exit_codes::CliError;
use crate::output::Output;
use engine::control;
use serde_json::json;
use std::ffi::OsString;

pub(crate) fn commands(args: Vec<OsString>) -> Result<Output, CliError> {
    if !args.is_empty() {
        return Err(CliError::Usage("usage: vibecut-cli commands".into()));
    }
    let registry = engine::registry();
    let value = json!({
        "api_version": control::API_VERSION,
        "registry_hash": control::registry_hash(),
        "limits": control::LIMITS,
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
    Ok(Output {
        value,
        human,
        silent: false,
        exit: 0,
    })
}

pub(crate) fn schema(args: Vec<OsString>) -> Result<Output, CliError> {
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
    Ok(Output {
        value,
        human,
        silent: false,
        exit: 0,
    })
}
