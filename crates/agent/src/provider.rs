//! Opens the chat adapter of the provider the user chose.
use crate::error::Error;
use ai::foundation::transport::ApiKey;
use ai::text::conversation::{AnthropicChat, ChatModel, OpenAiChat, default_model};
use std::sync::Arc;

/// A provider the agent can talk to. Chosen by the user; there is no fallback between them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Anthropic,
    OpenAi,
}
impl Provider {
    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "anthropic" => Some(Self::Anthropic),
            "openai" => Some(Self::OpenAi),
            _ => None,
        }
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
        }
    }
}

/// The adapter for `provider` with `key`, over the process HTTPS transport. `model` is the
/// user's setting; without one the provider's current tool-use model is used. Nothing is
/// sent here.
pub fn open_model(
    provider: Provider,
    model: Option<&str>,
    key: ApiKey,
) -> Result<Box<dyn ChatModel>, Error> {
    let model = match model {
        Some(model) => model,
        None => default_model(provider.id()).ok_or(ai::Error::Unsupported)?,
    };
    let transport = Arc::new(transport::UreqTransport::new());
    Ok(match provider {
        Provider::Anthropic => Box::new(AnthropicChat::new(
            AnthropicChat::provider_config(model)?,
            key,
            transport,
        )?),
        Provider::OpenAi => Box::new(OpenAiChat::new(
            OpenAiChat::provider_config(model)?,
            key,
            transport,
        )?),
    })
}
