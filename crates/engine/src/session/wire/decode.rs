//! Decodes a command id and its parameters against the registry schemas.
use super::budget;
use crate::{Command, EditError, Result, registry};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::OnceLock};

pub(crate) fn decode_command(id: &str, params: Value) -> Result<Command> {
    budget(&params)?;
    type Validators = BTreeMap<&'static str, jsonschema::Validator>;
    static VALIDATORS: OnceLock<std::result::Result<Validators, ()>> = OnceLock::new();
    let validators = VALIDATORS
        .get_or_init(|| {
            registry()
                .iter()
                .map(|s| {
                    jsonschema::options()
                        .offline()
                        .build(&s.params_schema)
                        .map(|v| (s.id, v))
                        .map_err(|_| ())
                })
                .collect()
        })
        .as_ref()
        .map_err(|_| super::ControlError::SchemaMismatch)?;
    let validator = validators.get(id).ok_or(EditError::UnknownCommand)?;
    if !validator.is_valid(&params) {
        return Err(EditError::InvalidParams);
    }
    let command: Command = serde_json::from_value(json!({"command":id,"params":params}))
        .map_err(|_| EditError::InvalidParams)?;
    // Includes nested publications/session controls, before any first child effect.
    if matches!(
        command,
        Command::Batch { .. } | Command::TimelineEdit { .. }
    ) {
        crate::session::check_budget(&command)?;
    }
    Ok(command)
}
