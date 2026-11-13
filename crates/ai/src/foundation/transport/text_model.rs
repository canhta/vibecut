//! Executes one bounded structured request against an admitted OpenAI or Anthropic text model.
//!
//! The owner supplies the operation prompt and schema; this module owns the provider
//! identity check, the exact wire body, the bounded send and the shared response
//! parsing. It never retries or switches provider.
use crate::foundation::transport::ApiKey;
use crate::{Error, ExecutionHooks, ProviderConfig, Stage, contracts::checkpoint, remote};
use ::transport::{HttpRequest, HttpResponse, HttpTransport};
use captions::NfcText;
use serde_json::{Value, json};
use std::sync::Arc;

/// The approved OpenAI text model shared by transcript-text operations.
pub(crate) const MAX_OUTPUT: u64 = 8192;
/// Wire ceiling shared by the transcript-text operations.
pub(crate) const MAX_WIRE: u64 = 131_072;

enum Client {
    OpenAi {
        key: ApiKey,
        transport: Arc<dyn HttpTransport>,
    },
    Anthropic(crate::providers::anthropic::MessagesClient),
}
pub(crate) struct TextModel {
    config: ProviderConfig,
    client: Client,
}
impl TextModel {
    /// Admit exactly one official provider configuration for this operation.
    pub(crate) fn new(
        config: ProviderConfig,
        key: ApiKey,
        transport: Arc<dyn HttpTransport>,
        anthropic: bool,
        settings: &str,
    ) -> Result<Self, Error> {
        let (backend, base) = if anthropic {
            (
                crate::providers::anthropic::BACKEND,
                crate::providers::anthropic::BASE_URL,
            )
        } else {
            (
                crate::providers::openai::BACKEND,
                crate::providers::openai::BASE_URL,
            )
        };
        remote::admit(&config, Some(settings), backend, base, None)?;
        let client = if anthropic {
            Client::Anthropic(crate::providers::anthropic::MessagesClient::new(
                key, transport,
            ))
        } else {
            Client::OpenAi { key, transport }
        };
        Ok(Self { config, client })
    }
    pub(crate) fn config(&self) -> &ProviderConfig {
        &self.config
    }
    pub(crate) fn provider(&self) -> &'static str {
        match self.client {
            Client::OpenAi { .. } => "OpenAI",
            Client::Anthropic(_) => "Anthropic",
        }
    }
    pub(crate) fn policy_url(&self) -> &'static str {
        match self.client {
            Client::OpenAi { .. } => "https://developers.openai.com/api/docs/guides/your-data",
            Client::Anthropic(_) => crate::providers::anthropic::POLICY_URL,
        }
    }
    /// Send one structured request and return only the decoded JSON object.
    pub(crate) fn send(&self, request: TextRequest<'_>) -> Result<Value, Error> {
        let TextRequest {
            name,
            instructions,
            schema,
            payload,
            stage,
            hooks,
            max_bytes,
        } = request;
        let body = match self.client {
            Client::OpenAi { .. } => {
                json!({"model":self.config.model().as_str(),"store":false,"background":false,"service_tier":"default","max_output_tokens":MAX_OUTPUT,"instructions":instructions,"input":payload.as_str(),"text":{"format":crate::text::structured::format(name,schema)}})
            }
            Client::Anthropic(_) => {
                json!({"model":self.config.model().as_str(),"max_tokens":MAX_OUTPUT,"stream":false,"thinking":{"type":"between_tools"},"system":instructions,"messages":[{"role":"user","content":payload.as_str()}],"output_config":{"format":{"type":"json_schema","schema":schema}}})
            }
        };
        let body = serde_json::to_vec(&body).map_err(|_| Error::Payload)?;
        if body.len() as u64 > MAX_WIRE || body.len() as u64 > max_bytes {
            return Err(Error::Payload);
        }
        checkpoint(hooks, stage)?;
        let response = match &self.client {
            Client::OpenAi { key, transport } => transport.send(HttpRequest {
                method: "POST",
                url: crate::providers::openai::TRANSLATION_ENDPOINT.into(),
                headers: vec![
                    ("Authorization".into(), format!("Bearer {}", key.expose())),
                    ("Content-Type".into(), "application/json".into()),
                ],
                body,
            })?,
            Client::Anthropic(client) => client.send(body)?,
        };
        remote::check_model_response(&response, &self.config)?;
        checkpoint(hooks, stage)?;
        match self.client {
            Client::OpenAi { .. } => decode_openai(&response),
            Client::Anthropic(_) => decode_anthropic(&response),
        }
    }
}

/// One bounded structured text request owned by the operation adapter.
pub(crate) struct TextRequest<'a> {
    pub name: &'a str,
    pub instructions: &'a str,
    pub schema: Value,
    pub payload: &'a NfcText,
    pub stage: Stage,
    pub hooks: &'a dyn ExecutionHooks,
    pub max_bytes: u64,
}
fn decode_openai(response: &HttpResponse) -> Result<Value, Error> {
    crate::text::structured::parse(response)
}
fn decode_anthropic(response: &HttpResponse) -> Result<Value, Error> {
    serde_json::from_str(&crate::providers::anthropic::message_text(response)?)
        .map_err(|_| Error::Schema)
}
