//! Plays a fixed script as a chat model, so callers of a conversation test without a network.
use super::{
    AnthropicChat, AssistantTurn, ChatEvent, ChatModel, ChatReply, ChatRequest, Message,
    PreparedRequest, Replay, ToolCall, Usage, default_model,
};
use crate::{Error, ExecutionHooks, ProviderConfig};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::Mutex;

/// What one scripted request answers with.
#[derive(Clone, Debug, Default)]
pub struct ScriptedReply {
    pub text: String,
    pub calls: Vec<ToolCall>,
}

/// Builds a reply from the messages the conversation sent, so a script can read the
/// result of its own earlier tool calls.
pub type Compose = Box<dyn FnOnce(&[Message]) -> ScriptedReply + Send>;

/// One request of the script, consumed in order.
pub enum ScriptedStep {
    Reply(ScriptedReply),
    Compose(Compose),
    /// The request fails with this error, as a provider failure would.
    Fail(Error),
}

/// A [`ChatModel`] that plays a fixed script. It speaks no network protocol: the request
/// body is the serialized history, so the conversation's ledger and context limits apply as
/// they do to a real adapter. A request after the script ends is `Incomplete`.
pub struct ScriptedChat {
    config: ProviderConfig,
    script: Mutex<VecDeque<ScriptedStep>>,
    seen: Mutex<Vec<Message>>,
    streams: Mutex<usize>,
}
impl ScriptedChat {
    /// The configuration a consent for this model must name: the default priced model of
    /// the first provider, so the ledger holds a rate for it.
    pub fn provider_config() -> Result<ProviderConfig, Error> {
        let model = default_model("anthropic").ok_or(Error::Unsupported)?;
        AnthropicChat::provider_config(model)
    }

    pub fn new(script: Vec<ScriptedStep>) -> Result<Self, Error> {
        Ok(Self {
            config: Self::provider_config()?,
            script: Mutex::new(script.into()),
            seen: Mutex::new(Vec::new()),
            streams: Mutex::new(0),
        })
    }

    /// Streams started so far, answered or failed.
    pub fn streams(&self) -> usize {
        self.streams.lock().map_or(0, |count| *count)
    }
}
impl ChatModel for ScriptedChat {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn provider(&self) -> &'static str {
        "Scripted"
    }
    fn links(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }
    fn prepare(&self, request: &ChatRequest<'_>) -> Result<PreparedRequest, Error> {
        let body =
            serde_json::to_vec(request.messages).map_err(|_| Error::Invalid("scripted request"))?;
        *self
            .seen
            .lock()
            .map_err(|_| Error::Invalid("scripted request"))? = request.messages.to_vec();
        Ok(PreparedRequest { body })
    }
    fn stream(
        &self,
        _request: PreparedRequest,
        on_event: &mut dyn FnMut(ChatEvent<'_>),
        hooks: &dyn ExecutionHooks,
    ) -> Result<ChatReply, Error> {
        if let Ok(mut count) = self.streams.lock() {
            *count += 1;
        }
        if hooks.cancelled() {
            hooks.cancelling();
            return Err(Error::Cancelled);
        }
        let step = self
            .script
            .lock()
            .map_err(|_| Error::Incomplete)?
            .pop_front()
            .ok_or(Error::Incomplete)?;
        let reply = match step {
            ScriptedStep::Reply(reply) => reply,
            ScriptedStep::Compose(compose) => {
                let seen = self.seen.lock().map_err(|_| Error::Incomplete)?.clone();
                compose(&seen)
            }
            ScriptedStep::Fail(error) => return Err(error),
        };
        if !reply.text.is_empty() {
            on_event(ChatEvent::TextDelta(&reply.text));
        }
        for call in &reply.calls {
            on_event(ChatEvent::ToolCall(call));
        }
        Ok(ChatReply {
            assistant: AssistantTurn::new(
                reply.text,
                reply.calls,
                Replay::new("scripted", Value::Null),
            ),
            usage: Some(Usage {
                input_tokens: 100,
                output_tokens: 20,
            }),
        })
    }
}
