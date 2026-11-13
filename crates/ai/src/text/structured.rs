//! Shared strict Responses JSON parsing; operation owners validate semantic IDs.
use crate::Error;
use ::transport::{HttpResponse, MAX_RESPONSE_BYTES};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
/// Schema objects must enumerate required fields and reject additional properties.
pub fn format(name: &str, schema: Value) -> Value {
    json!({"type":"json_schema", "name":name, "strict":true, "schema":schema})
}
/// Refusals, unfinished replies and unexpected tool/text payloads never become
/// successful structured data. Bodies are bounded and diagnostics redact user text.
pub fn parse<T: DeserializeOwned>(response: &HttpResponse) -> Result<T, Error> {
    if response.body.len() > MAX_RESPONSE_BYTES {
        return Err(Error::Payload);
    }
    if !(200..=299).contains(&response.status) {
        return Err(crate::providers::openai::status_error(response));
    }
    let value: Value = serde_json::from_slice(&response.body).map_err(|_| Error::Schema)?;
    if value.get("status").and_then(Value::as_str) != Some("completed") {
        return Err(Error::Incomplete);
    }
    let output = value
        .get("output")
        .and_then(Value::as_array)
        .ok_or(Error::Schema)?;
    let mut text = None;
    for item in output {
        match item.get("type").and_then(Value::as_str) {
            Some("reasoning") => continue,
            Some("message") => {}
            _ => return Err(Error::Schema),
        }
        if item.get("role").and_then(Value::as_str) != Some("assistant")
            || item.get("status").and_then(Value::as_str) != Some("completed")
        {
            return Err(Error::Schema);
        }
        for content in item
            .get("content")
            .and_then(Value::as_array)
            .ok_or(Error::Schema)?
        {
            match content.get("type").and_then(Value::as_str) {
                Some("refusal") => return Err(Error::Refused),
                Some("output_text") if text.is_none() => {
                    text = Some(
                        content
                            .get("text")
                            .and_then(Value::as_str)
                            .ok_or(Error::Schema)?,
                    )
                }
                _ => return Err(Error::Schema),
            }
        }
    }
    serde_json::from_str(text.ok_or(Error::Schema)?).map_err(|_| Error::Schema)
}
