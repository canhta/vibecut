//! Fetches bounded remote model identifiers only on an explicit caller request.
use crate::transport::ApiKey;
use crate::{Error, ExecutionHooks, defaults::validate_model};
use ::transport::{HttpRequest, HttpTransport, MAX_RESPONSE_BYTES};
use captions::NfcText;
use std::collections::BTreeSet;

pub const MAX_REMOTE_MODELS: usize = 1000;

/// One read-only list request, with the selected key and an empty body. No user
/// content, project data or model download is sent. Call only for explicit Refresh.
pub fn list_remote_models(
    provider: &str,
    key: &ApiKey,
    transport: &dyn HttpTransport,
    hooks: &dyn ExecutionHooks,
) -> Result<Vec<NfcText>, Error> {
    let (url, headers) = match provider {
        "openai" => (
            "https://api.openai.com/v1/models",
            vec![("Authorization".into(), format!("Bearer {}", key.expose()))],
        ),
        "anthropic" => (
            "https://api.anthropic.com/v1/models?limit=1000",
            vec![
                ("x-api-key".into(), key.expose().into()),
                (
                    "anthropic-version".into(),
                    crate::anthropic::ANTHROPIC_VERSION.into(),
                ),
            ],
        ),
        "elevenlabs" => (
            "https://api.elevenlabs.io/v1/models",
            vec![("xi-api-key".into(), key.expose().into())],
        ),
        _ => return Err(Error::Unsupported),
    };
    if hooks.cancelled() {
        return Err(Error::Cancelled);
    }
    let response = transport.send(HttpRequest {
        method: "GET",
        url: url.into(),
        headers,
        body: Vec::new(),
    })?;
    if hooks.cancelled() {
        return Err(Error::Cancelled);
    }
    if response.status != 200 {
        return Err(crate::remote::status_error(&response, |_, _| None));
    }
    if response.body.len() > MAX_RESPONSE_BYTES {
        return Err(Error::Payload);
    }
    let value: serde_json::Value =
        serde_json::from_slice(&response.body).map_err(|_| Error::Schema)?;
    let rows = if provider == "elevenlabs" {
        value.as_array()
    } else {
        value.get("data").and_then(serde_json::Value::as_array)
    }
    .ok_or(Error::Schema)?;
    if rows.len() > MAX_REMOTE_MODELS {
        return Err(Error::Payload);
    }
    let mut ids = BTreeSet::new();
    for row in rows {
        let id = row
            .get(if provider == "elevenlabs" {
                "model_id"
            } else {
                "id"
            })
            .and_then(serde_json::Value::as_str)
            .ok_or(Error::Schema)?;
        validate_model(id).map_err(|_| Error::Schema)?;
        ids.insert(id.to_owned());
    }
    Ok(ids.into_iter().map(NfcText::from).collect())
}
