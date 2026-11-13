//! OpenAI Responses streaming with function tools behind [`ChatModel`].
//! Sources checked 2026-11-12: Responses streaming events (`response.output_item.added`,
//! `response.output_text.delta`, `response.function_call_arguments.delta|done`,
//! `response.output_item.done`, `response.completed|incomplete|failed`, `error`),
//! `function_call_output` input items keyed by `call_id`, and stateless use with
//! `store=false` plus `include: reasoning.encrypted_content` so reasoning items can
//! be passed back (OpenAI function-calling and streaming guides, GPT-6.1 Sol model page).
use super::sse::{self, Sse};
use super::{
    AssistantTurn, ChatEvent, ChatModel, ChatReply, ChatRequest, MAX_REPLY_BYTES, Message,
    PreparedRequest, Replay, ToolCall, ToolInput, Usage,
};
use crate::foundation::transport::ApiKey;
use crate::remote;
use crate::{Error, ExecutionHooks, ProviderConfig};
use ::transport::{HttpRequest, StreamTransport};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

const BACKEND: &str = "openai";
const SETTINGS: &str = "openai-agent-chat-v1;responses;stream;store=false;tools;parallel";
const MAX_ITEMS: usize = 128;
const LINKS: &[(&str, &str)] = &[
    (
        "policy",
        "https://developers.openai.com/api/docs/guides/your-data",
    ),
    ("pricing", "https://developers.openai.com/api/docs/pricing"),
    (
        "function_calling",
        "https://developers.openai.com/api/docs/guides/function-calling",
    ),
];

/// Agent chat over the OpenAI Responses API.
pub struct OpenAiChat {
    config: ProviderConfig,
    key: ApiKey,
    transport: Arc<dyn StreamTransport>,
}
impl OpenAiChat {
    /// The one admitted configuration for this model id.
    pub fn provider_config(model: &str) -> Result<ProviderConfig, Error> {
        remote::provider_config(
            model,
            SETTINGS,
            BACKEND,
            crate::providers::openai::BASE_URL,
            "openai",
            "OPENAI_API_KEY",
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
            crate::providers::openai::BASE_URL,
            None,
        )?;
        Ok(Self {
            config,
            key,
            transport,
        })
    }
}
impl fmt::Debug for OpenAiChat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiChat")
            .field("config", &self.config)
            .field("key", &self.key)
            .field("transport", &"[transport]")
            .finish()
    }
}

impl ChatModel for OpenAiChat {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn provider(&self) -> &'static str {
        "OpenAI"
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
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                    // Registry schemas are not guaranteed to meet strict mode's rules;
                    // arguments are validated by the tool owner.
                    "strict": false,
                })
            })
            .collect();
        let mut body = json!({
            "model": self.config.model().as_str(),
            "store": false,
            "background": false,
            "stream": true,
            "max_output_tokens": request.max_output_tokens,
            "parallel_tool_calls": true,
            "include": ["reasoning.encrypted_content"],
            "instructions": request.system,
            "input": input(request.messages)?,
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
                url: crate::providers::openai::TRANSLATION_ENDPOINT.into(),
                headers: vec![
                    (
                        "Authorization".into(),
                        format!("Bearer {}", self.key.expose()),
                    ),
                    ("Content-Type".into(), "application/json".into()),
                    ("Accept".into(), "text/event-stream".into()),
                ],
                body: request.body,
            },
            &self.config,
            crate::providers::openai::status_error,
        )?;
        read(&mut events, on_event, hooks)
    }
}

fn input(history: &[Message]) -> Result<Vec<Value>, Error> {
    let mut items = Vec::new();
    for message in history {
        match message {
            Message::User(text) => items.push(json!({"role": "user", "content": text})),
            Message::Assistant(turn) => match turn.replay().items(BACKEND)? {
                Value::Array(recorded) => items.extend(recorded.iter().cloned()),
                _ => return Err(Error::Invalid("conversation replay")),
            },
            Message::ToolResults(results) => {
                for result in results {
                    // Responses has no error flag on a function output.
                    let output = if result.is_error {
                        format!("error: {}", result.content)
                    } else {
                        result.content.clone()
                    };
                    items.push(json!({
                        "type": "function_call_output",
                        "call_id": result.call_id,
                        "output": output,
                    }));
                }
            }
        }
    }
    Ok(items)
}

/// Fold one stream into the assistant turn. Every provider event is checked for
/// cancellation first; a stream that ends before `response.completed` is incomplete.
fn read(
    events: &mut Sse,
    on_event: &mut dyn FnMut(ChatEvent<'_>),
    hooks: &dyn ExecutionHooks,
) -> Result<ChatReply, Error> {
    let mut items: BTreeMap<u64, Value> = BTreeMap::new();
    let mut calls: Vec<ToolCall> = Vec::new();
    let mut text = String::new();
    let mut bytes = 0usize;
    let mut usage = None;
    let mut completed = false;
    while let Some(data) = events.next_event()? {
        if hooks.cancelled() {
            hooks.cancelling();
            return Err(Error::Cancelled);
        }
        if data.trim() == "[DONE]" {
            break;
        }
        let event: Value = serde_json::from_str(&data).map_err(|_| Error::Schema)?;
        match event.get("type").and_then(Value::as_str) {
            Some("response.output_text.delta") => {
                let piece = event
                    .get("delta")
                    .and_then(Value::as_str)
                    .ok_or(Error::Schema)?;
                bytes = bytes.checked_add(piece.len()).ok_or(Error::Payload)?;
                if bytes > MAX_REPLY_BYTES {
                    return Err(Error::Payload);
                }
                text.push_str(piece);
                on_event(ChatEvent::TextDelta(piece));
            }
            Some("response.refusal.delta" | "response.refusal.done") => {
                return Err(Error::Refused);
            }
            Some("response.output_item.done") => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .ok_or(Error::Schema)?;
                let item = event.get("item").ok_or(Error::Schema)?;
                if items.len() >= MAX_ITEMS {
                    return Err(Error::Payload);
                }
                let recorded = match item.get("type").and_then(Value::as_str) {
                    Some("function_call") => {
                        let arguments = item
                            .get("arguments")
                            .and_then(Value::as_str)
                            .ok_or(Error::Schema)?;
                        bytes = bytes.checked_add(arguments.len()).ok_or(Error::Payload)?;
                        if bytes > MAX_REPLY_BYTES {
                            return Err(Error::Payload);
                        }
                        let parsed = parse_arguments(arguments);
                        let call = ToolCall {
                            id: string(item, "call_id")?,
                            name: string(item, "name")?,
                            input: parsed,
                        };
                        on_event(ChatEvent::ToolCall(&call));
                        // A malformed argument string is replayed as an empty object.
                        let mut recorded = item.clone();
                        if call.input == ToolInput::Malformed
                            && let Some(object) = recorded.as_object_mut()
                        {
                            object.insert("arguments".into(), "{}".into());
                        }
                        calls.push(call);
                        recorded
                    }
                    Some("message") => {
                        refuse_if_refusal(item)?;
                        item.clone()
                    }
                    Some("reasoning") => item.clone(),
                    _ => return Err(Error::Schema),
                };
                items.insert(index, recorded);
            }
            Some("response.completed") => {
                let response = event.get("response").ok_or(Error::Schema)?;
                if let Some(used) = response.get("usage").filter(|usage| !usage.is_null()) {
                    usage = Some(Usage {
                        input_tokens: used
                            .get("input_tokens")
                            .and_then(Value::as_u64)
                            .ok_or(Error::Schema)?,
                        output_tokens: used
                            .get("output_tokens")
                            .and_then(Value::as_u64)
                            .ok_or(Error::Schema)?,
                    });
                }
                completed = true;
                break;
            }
            Some("response.incomplete") => return Err(Error::Incomplete),
            Some("response.failed") => {
                let error = event.get("response").and_then(|r| r.get("error"));
                return Err(failure(error.and_then(|e| e.get("code"))));
            }
            Some("error") => return Err(failure(event.get("code"))),
            // Created, in_progress, content_part and reasoning-summary events carry
            // nothing the turn needs; the done events above are authoritative.
            _ => {}
        }
    }
    if !completed {
        return Err(Error::Incomplete);
    }
    if items.is_empty() {
        return Err(Error::Incomplete);
    }
    let replay = Replay::new(BACKEND, Value::Array(items.into_values().collect()));
    Ok(ChatReply {
        assistant: AssistantTurn::new(text, calls, replay),
        usage,
    })
}

fn refuse_if_refusal(message: &Value) -> Result<(), Error> {
    let refused = message
        .get("content")
        .and_then(Value::as_array)
        .is_some_and(|parts| {
            parts
                .iter()
                .any(|part| part.get("type").and_then(Value::as_str) == Some("refusal"))
        });
    if refused { Err(Error::Refused) } else { Ok(()) }
}
fn string(value: &Value, name: &str) -> Result<String, Error> {
    value
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(Error::Schema)
}
/// An empty string is a call with no arguments; anything else must parse to an object.
fn parse_arguments(arguments: &str) -> ToolInput {
    let text = if arguments.trim().is_empty() {
        "{}"
    } else {
        arguments
    };
    match serde_json::from_str::<Value>(text) {
        Ok(value @ Value::Object(_)) => ToolInput::Valid(value),
        _ => ToolInput::Malformed,
    }
}
/// Classify only the error `code`; the message text can quote the request and is dropped.
fn failure(code: Option<&Value>) -> Error {
    match code.and_then(Value::as_str) {
        Some("rate_limit_exceeded") => Error::RateLimit {
            retry_after_seconds: None,
        },
        Some("insufficient_quota") => Error::Quota,
        Some("invalid_prompt" | "invalid_request_error") => {
            Error::Invalid("provider rejected request")
        }
        _ => Error::Server,
    }
}
