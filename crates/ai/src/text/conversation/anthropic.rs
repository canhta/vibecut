//! Anthropic Messages streaming with tool use behind [`ChatModel`].
//! Sources checked 2026-11-12: streaming events (`message_start`, `content_block_*`,
//! `message_delta`, `message_stop`, `ping`, `error`), `input_json_delta` partial JSON
//! per tool_use block, `tool_result` blocks in the next user message and thinking
//! blocks passed back unmodified, and `image` blocks with a `base64` source inside a
//! `tool_result`'s content array, in the Messages API reference.
use super::sse::{self, Sse};
use super::{
    AssistantTurn, ChatEvent, ChatModel, ChatReply, ChatRequest, MAX_REPLY_BYTES, Message,
    PreparedRequest, Replay, ToolCall, ToolInput, ToolResult, Usage,
};
use crate::foundation::transport::ApiKey;
use crate::remote;
use crate::{Error, ExecutionHooks, ProviderConfig};
use ::transport::{HttpRequest, StreamTransport};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

const BACKEND: &str = "anthropic";
const SETTINGS: &str = "anthropic-agent-chat-v1;messages;stream;tools;thinking=between_tools";
const MAX_BLOCKS: usize = 64;
const LINKS: &[(&str, &str)] = &[
    ("policy", crate::providers::anthropic::POLICY_URL),
    (
        "pricing",
        "https://platform.claude.com/docs/en/about-claude/pricing",
    ),
    (
        "tool_use",
        "https://platform.claude.com/docs/en/agents-and-tools/tool-use/overview",
    ),
];

/// Agent chat over the Anthropic Messages API.
pub struct AnthropicChat {
    config: ProviderConfig,
    key: ApiKey,
    transport: Arc<dyn StreamTransport>,
}
impl AnthropicChat {
    /// The one admitted configuration for this model id.
    pub fn provider_config(model: &str) -> Result<ProviderConfig, Error> {
        remote::provider_config(
            model,
            SETTINGS,
            BACKEND,
            crate::providers::anthropic::BASE_URL,
            "anthropic",
            "ANTHROPIC_API_KEY",
        )
    }
    pub fn new(
        config: ProviderConfig,
        key: ApiKey,
        transport: Arc<dyn StreamTransport>,
    ) -> Result<Self, Error> {
        remote::admit(
            &config,
            Some(SETTINGS),
            BACKEND,
            crate::providers::anthropic::BASE_URL,
            None,
        )?;
        Ok(Self {
            config,
            key,
            transport,
        })
    }
}
impl fmt::Debug for AnthropicChat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicChat")
            .field("config", &self.config)
            .field("key", &self.key)
            .field("transport", &"[transport]")
            .finish()
    }
}

impl ChatModel for AnthropicChat {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn provider(&self) -> &'static str {
        "Anthropic"
    }
    fn links(&self) -> &'static [(&'static str, &'static str)] {
        LINKS
    }
    fn prepare(&self, request: &ChatRequest<'_>) -> Result<PreparedRequest, Error> {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.input_schema,
                })
            })
            .collect();
        let mut body = json!({
            "model": self.config.model().as_str(),
            "max_tokens": request.max_output_tokens,
            "stream": true,
            "thinking": {"type": "between_tools"},
            "system": request.system,
            "messages": messages(request.messages)?,
        });
        if !tools.is_empty()
            && let Some(object) = body.as_object_mut()
        {
            object.insert("tools".into(), Value::Array(tools));
        }
        Ok(PreparedRequest {
            body: serde_json::to_vec(&body).map_err(|_| Error::Payload)?,
        })
    }
    fn stream(
        &self,
        request: PreparedRequest,
        on_event: &mut dyn FnMut(ChatEvent<'_>),
        hooks: &dyn ExecutionHooks,
    ) -> Result<ChatReply, Error> {
        let mut events = sse::open(
            self.transport.as_ref(),
            HttpRequest {
                method: "POST",
                url: crate::providers::anthropic::ENDPOINT.into(),
                headers: vec![
                    ("x-api-key".into(), self.key.expose().to_owned()),
                    (
                        "anthropic-version".into(),
                        crate::providers::anthropic::ANTHROPIC_VERSION.into(),
                    ),
                    ("Content-Type".into(), "application/json".into()),
                    ("Accept".into(), "text/event-stream".into()),
                ],
                body: request.body,
            },
            &self.config,
            crate::providers::anthropic::status_error,
        )?;
        read(&mut events, on_event, hooks)
    }
}

fn messages(history: &[Message]) -> Result<Vec<Value>, Error> {
    history
        .iter()
        .map(|message| match message {
            Message::User(text) => Ok(json!({"role": "user", "content": text})),
            Message::Assistant(turn) => {
                Ok(json!({"role": "assistant", "content": turn.replay().items(BACKEND)?.clone()}))
            }
            Message::ToolResults(results) => {
                let blocks: Vec<Value> = results
                    .iter()
                    .map(|result| {
                        let mut block = Map::new();
                        block.insert("type".into(), "tool_result".into());
                        block.insert("tool_use_id".into(), result.call_id.clone().into());
                        block.insert("content".into(), result_content(result));
                        if result.is_error {
                            block.insert("is_error".into(), true.into());
                        }
                        Value::Object(block)
                    })
                    .collect();
                Ok(json!({"role": "user", "content": blocks}))
            }
        })
        .collect()
}

/// A plain string, or text then image blocks when the result shows the model frames.
fn result_content(result: &ToolResult) -> Value {
    if result.images.is_empty() {
        return result.content.clone().into();
    }
    let mut blocks = vec![json!({"type": "text", "text": result.content})];
    blocks.extend(result.images.iter().map(|image| {
        json!({"type": "image", "source": {
            "type": "base64",
            "media_type": image.image_type().media_type(),
            "data": image.base64(),
        }})
    }));
    Value::Array(blocks)
}

enum Block {
    Text(String),
    Thinking {
        text: String,
        signature: String,
    },
    Redacted(Value),
    Tool {
        id: String,
        name: String,
        json: String,
    },
}

/// Fold one stream into the assistant turn. Every provider event is checked for
/// cancellation first; a stream that ends before `message_stop` is incomplete.
fn read(
    events: &mut Sse,
    on_event: &mut dyn FnMut(ChatEvent<'_>),
    hooks: &dyn ExecutionHooks,
) -> Result<ChatReply, Error> {
    let mut open: BTreeMap<u64, Block> = BTreeMap::new();
    let mut content: BTreeMap<u64, Value> = BTreeMap::new();
    let mut calls: Vec<ToolCall> = Vec::new();
    let mut text = String::new();
    let mut bytes = 0usize;
    let mut usage = Usage::default();
    let mut stop = None;
    let mut stopped = false;
    while let Some(data) = events.next_event()? {
        if hooks.cancelled() {
            hooks.cancelling();
            return Err(Error::Cancelled);
        }
        let event: Value = serde_json::from_str(&data).map_err(|_| Error::Schema)?;
        let index = event.get("index").and_then(Value::as_u64);
        match event.get("type").and_then(Value::as_str) {
            Some("ping") => {}
            Some("message_start") => {
                if let Some(start) = event.get("message").and_then(|m| m.get("usage")) {
                    usage.input_tokens = input_tokens(start);
                    usage.output_tokens = start
                        .get("output_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                }
            }
            Some("content_block_start") => {
                if open.len() + content.len() >= MAX_BLOCKS {
                    return Err(Error::Payload);
                }
                let block = event.get("content_block").ok_or(Error::Schema)?;
                let started = match block.get("type").and_then(Value::as_str) {
                    Some("text") => Block::Text(String::new()),
                    Some("thinking") => Block::Thinking {
                        text: String::new(),
                        signature: String::new(),
                    },
                    Some("redacted_thinking") => Block::Redacted(block.clone()),
                    Some("tool_use") => Block::Tool {
                        id: string(block, "id")?,
                        name: string(block, "name")?,
                        json: String::new(),
                    },
                    _ => return Err(Error::Schema),
                };
                if open.insert(index.ok_or(Error::Schema)?, started).is_some() {
                    return Err(Error::Schema);
                }
            }
            Some("content_block_delta") => {
                let delta = event.get("delta").ok_or(Error::Schema)?;
                let block = open
                    .get_mut(&index.ok_or(Error::Schema)?)
                    .ok_or(Error::Schema)?;
                match (delta.get("type").and_then(Value::as_str), block) {
                    (Some("text_delta"), Block::Text(buffer)) => {
                        let piece = str_field(delta, "text")?;
                        grow(&mut bytes, piece)?;
                        buffer.push_str(piece);
                        text.push_str(piece);
                        on_event(ChatEvent::TextDelta(piece));
                    }
                    (Some("input_json_delta"), Block::Tool { json, .. }) => {
                        let piece = str_field(delta, "partial_json")?;
                        grow(&mut bytes, piece)?;
                        json.push_str(piece);
                    }
                    (Some("thinking_delta"), Block::Thinking { text, .. }) => {
                        let piece = str_field(delta, "thinking")?;
                        grow(&mut bytes, piece)?;
                        text.push_str(piece);
                    }
                    (Some("signature_delta"), Block::Thinking { signature, .. }) => {
                        let piece = str_field(delta, "signature")?;
                        grow(&mut bytes, piece)?;
                        signature.push_str(piece);
                    }
                    _ => return Err(Error::Schema),
                }
            }
            Some("content_block_stop") => {
                let index = index.ok_or(Error::Schema)?;
                match open.remove(&index).ok_or(Error::Schema)? {
                    Block::Text(buffer) => {
                        // The API rejects empty text blocks on replay.
                        if !buffer.is_empty() {
                            content.insert(index, json!({"type": "text", "text": buffer}));
                        }
                    }
                    Block::Thinking { text, signature } => {
                        content.insert(
                            index,
                            json!({"type": "thinking", "thinking": text, "signature": signature}),
                        );
                    }
                    Block::Redacted(block) => {
                        content.insert(index, block);
                    }
                    Block::Tool { id, name, json } => {
                        let parsed = parse_arguments(&json);
                        let replay_input = match &parsed {
                            ToolInput::Valid(value) => value.clone(),
                            ToolInput::Malformed => json!({}),
                        };
                        content.insert(
                            index,
                            json!({"type": "tool_use", "id": id, "name": name, "input": replay_input}),
                        );
                        let call = ToolCall {
                            id,
                            name,
                            input: parsed,
                        };
                        on_event(ChatEvent::ToolCall(&call));
                        calls.push(call);
                    }
                }
            }
            Some("message_delta") => {
                if let Some(delta) = event.get("delta") {
                    if delta
                        .get("stop_details")
                        .and_then(|details| details.get("type"))
                        .and_then(Value::as_str)
                        == Some("refusal")
                    {
                        return Err(Error::Refused);
                    }
                    if let Some(reason) = delta.get("stop_reason").and_then(Value::as_str) {
                        stop = Some(reason.to_owned());
                    }
                }
                if let Some(delta) = event.get("usage") {
                    // Cumulative counts; input is absent from some deltas.
                    if delta.get("input_tokens").is_some_and(|v| !v.is_null()) {
                        usage.input_tokens = input_tokens(delta);
                    }
                    if let Some(output) = delta.get("output_tokens").and_then(Value::as_u64) {
                        usage.output_tokens = output;
                    }
                }
            }
            Some("message_stop") => {
                stopped = true;
                break;
            }
            Some("error") => return Err(stream_error(&event)),
            // New event types are ignored per the streaming guidance on versioning.
            _ => {}
        }
    }
    if !stopped || !open.is_empty() {
        return Err(Error::Incomplete);
    }
    match stop.as_deref() {
        Some("end_turn" | "tool_use") => {}
        Some("refusal") => return Err(Error::Refused),
        _ => return Err(Error::Incomplete),
    }
    // An assistant message with no content cannot be replayed.
    if content.is_empty() {
        return Err(Error::Incomplete);
    }
    let replay = Replay::new(BACKEND, Value::Array(content.into_values().collect()));
    Ok(ChatReply {
        assistant: AssistantTurn::new(text, calls, replay),
        usage: Some(usage),
    })
}

fn grow(total: &mut usize, piece: &str) -> Result<(), Error> {
    *total = total.checked_add(piece.len()).ok_or(Error::Payload)?;
    if *total > MAX_REPLY_BYTES {
        return Err(Error::Payload);
    }
    Ok(())
}
fn str_field<'a>(value: &'a Value, name: &str) -> Result<&'a str, Error> {
    value.get(name).and_then(Value::as_str).ok_or(Error::Schema)
}
fn string(value: &Value, name: &str) -> Result<String, Error> {
    str_field(value, name).map(str::to_owned)
}
/// Total input is the sum of the three counters.
fn input_tokens(usage: &Value) -> u64 {
    [
        "input_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
    ]
    .iter()
    .filter_map(|name| usage.get(*name).and_then(Value::as_u64))
    .fold(0, u64::saturating_add)
}
/// An empty buffer is a call with no arguments; anything else must parse to an object.
fn parse_arguments(buffer: &str) -> ToolInput {
    let text = if buffer.trim().is_empty() {
        "{}"
    } else {
        buffer
    };
    match serde_json::from_str::<Value>(text) {
        Ok(value @ Value::Object(_)) => ToolInput::Valid(value),
        _ => ToolInput::Malformed,
    }
}
/// Classify only the error `type`; the message text can quote the request and is dropped.
fn stream_error(event: &Value) -> Error {
    let kind = event
        .get("error")
        .and_then(|error| error.get("type"))
        .and_then(Value::as_str);
    match kind {
        Some("rate_limit_error") => Error::RateLimit {
            retry_after_seconds: None,
        },
        Some("authentication_error" | "permission_error") => Error::Auth,
        Some("billing_error") => Error::Quota,
        Some("request_too_large") => Error::Payload,
        Some("invalid_request_error") => Error::Invalid("provider rejected request"),
        Some("timeout_error") => Error::Timeout,
        _ => Error::Server,
    }
}
