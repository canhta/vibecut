//! Executes one bounded structured request against an admitted OpenAI or Anthropic text model.
//!
//! The owner supplies the operation prompt and schema; this module owns the provider
//! identity check, the exact wire body, the bounded send and the shared response
//! parsing. It never retries or switches provider.
use crate::transport::{ApiKey, HttpRequest, HttpTransport};
use crate::{
    Error, ExecutionHooks, Location, ProviderConfig, Stage, contracts::checkpoint,
    transport::HttpResponse,
};
use captions::NfcText;
use serde_json::{Value, json};
use std::sync::Arc;

/// The approved OpenAI text model shared by transcript-text operations.
pub(crate) const OPENAI_MODEL: &str = "gpt-6-luna";
pub(crate) const MAX_OUTPUT: u64 = 8192;
/// Wire ceiling shared by the transcript-text operations.
pub(crate) const MAX_WIRE: u64 = 131_072;

enum Client {
    OpenAi {
        key: ApiKey,
        transport: Arc<dyn HttpTransport>,
    },
    Anthropic(crate::anthropic::MessagesClient),
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
        model: &str,
        settings: &str,
    ) -> Result<Self, Error> {
        let (backend, base) = if anthropic {
            (crate::anthropic::BACKEND, crate::anthropic::BASE_URL)
        } else {
            (crate::openai::BACKEND, crate::openai::BASE_URL)
        };
        if config.model().as_str() != model || config.settings_fingerprint().as_str() != settings {
            return Err(Error::Unsupported);
        }
        match config.location() {
            Location::Remote {
                backend: b,
                base_url,
                ..
            } if b.as_str() == backend && base_url.as_str().trim_end_matches('/') == base => {}
            _ => return Err(Error::Unsupported),
        }
        let client = if anthropic {
            Client::Anthropic(crate::anthropic::MessagesClient::new(key, transport))
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
    pub(crate) fn endpoint(&self) -> &'static str {
        match self.client {
            Client::OpenAi { .. } => crate::openai::TRANSLATION_ENDPOINT,
            Client::Anthropic(_) => crate::anthropic::ENDPOINT,
        }
    }
    pub(crate) fn policy_url(&self) -> &'static str {
        match self.client {
            Client::OpenAi { .. } => "https://developers.openai.com/api/docs/guides/your-data",
            Client::Anthropic(_) => crate::anthropic::POLICY_URL,
        }
    }
    /// Send one structured request and return only the decoded JSON object.
    pub(crate) fn send(
        &self,
        name: &str,
        instructions: &str,
        schema: Value,
        payload: &NfcText,
        stage: Stage,
        hooks: &dyn ExecutionHooks,
        max_bytes: u64,
    ) -> Result<Value, Error> {
        let body = match self.client {
            Client::OpenAi { .. } => {
                json!({"model":self.config.model().as_str(),"store":false,"background":false,"service_tier":"default","max_output_tokens":MAX_OUTPUT,"instructions":instructions,"input":payload.as_str(),"text":{"format":crate::structured::format(name,schema)}})
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
                url: crate::openai::TRANSLATION_ENDPOINT.into(),
                headers: vec![
                    ("Authorization".into(), format!("Bearer {}", key.expose())),
                    ("Content-Type".into(), "application/json".into()),
                ],
                body,
            })?,
            Client::Anthropic(client) => client.send(body)?,
        };
        checkpoint(hooks, stage)?;
        match self.client {
            Client::OpenAi { .. } => decode_openai(&response),
            Client::Anthropic(_) => decode_anthropic(&response),
        }
    }
}
fn decode_openai(response: &HttpResponse) -> Result<Value, Error> {
    crate::structured::parse(response)
}
fn decode_anthropic(response: &HttpResponse) -> Result<Value, Error> {
    serde_json::from_str(&crate::anthropic::message_text(response)?).map_err(|_| Error::Schema)
}
