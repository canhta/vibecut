//! Opens the chat model of the provider the creator chose, and says whether its key exists.
use agent::Provider;
use ai::text::conversation::ChatModel;
use engine::providers::Credentials;

/// The seam to the provider adapters. The shipped one reads the key from the process
/// environment; tests lend a scripted model.
pub trait Backend: Send + Sync {
    /// Whether a usable key for `provider` is configured.
    fn has_key(&self, provider: Provider) -> bool;
    /// The chat model for `provider` at its current default version. Nothing is sent here.
    fn open(&self, provider: Provider) -> Result<Box<dyn ChatModel>, agent::Error>;
}

/// Keys from `ANTHROPIC_API_KEY` and `OPENAI_API_KEY`, like every other provider feature.
pub struct Environment;

impl Backend for Environment {
    fn has_key(&self, provider: Provider) -> bool {
        let credentials = Credentials::from_env();
        match provider {
            Provider::Anthropic => credentials.anthropic().is_ok(),
            Provider::OpenAi => credentials.openai().is_ok(),
        }
    }

    fn open(&self, provider: Provider) -> Result<Box<dyn ChatModel>, agent::Error> {
        let credentials = Credentials::from_env();
        let key = match provider {
            Provider::Anthropic => credentials.anthropic(),
            Provider::OpenAi => credentials.openai(),
        }
        .map_err(|_| agent::Error::Chat(ai::Error::Auth))?;
        agent::open_model(provider, None, key)
    }
}
