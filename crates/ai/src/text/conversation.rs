//! A streamed tool-use conversation with Anthropic or OpenAI behind one trait,
//! the agent-chat consent scope and the per-conversation spend ledger. No
//! provider type crosses this module's public surface; replay state the model
//! needs back (thinking blocks, reasoning items) travels as an opaque value only
//! the adapter that wrote it reads.
mod anthropic;
mod ledger;
mod openai;
#[cfg(feature = "test-chat")]
mod scripted;
mod sse;

use crate::{Error, ExecutionHooks, Money, ProviderConfig};
pub use anthropic::AnthropicChat;
use captions::NfcText;
pub use ledger::{Ledger, default_model};
pub use openai::OpenAiChat;
#[cfg(feature = "test-chat")]
pub use scripted::{Compose, ScriptedChat, ScriptedReply, ScriptedStep};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// A token is at least one byte, so a request body of at most this many bytes
/// stays under the 272K-token prompt tier both providers price higher above.
pub const MAX_CONTEXT_BYTES: u64 = 272_000;
pub const MAX_OUTPUT_TOKENS: u32 = 16_384;
pub const MAX_TOOLS: usize = 64;
/// Text plus tool-call bytes one reply may carry before the stream is refused.
pub(crate) const MAX_REPLY_BYTES: usize = 1024 * 1024;
const MALFORMED: &str = "The tool arguments were not a valid JSON object. Call the tool again with arguments that match its schema.";
const UNKNOWN_TOOL: &str = "No such tool. Call one of the listed tools.";

/// Kinds of project data a conversation may send, shown before the first
/// request. The user's own prompt is always sent and is not a kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataKind {
    ProjectSummary,
    ClipNames,
    TranscriptText,
    Frames,
}

/// What one conversation's consent binds: provider and model (inside the
/// configuration), the kinds of data that may leave and the spend limit in USD.
/// A change of any of them is a different scope and needs a new consent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentChatScope {
    pub provider: ProviderConfig,
    pub kinds: BTreeSet<DataKind>,
    pub limit: Money,
}
impl AgentChatScope {
    fn validate(&self) -> Result<(), Error> {
        if !self.provider.is_remote() {
            return Err(Error::Invalid("agent chat needs a remote provider"));
        }
        if self.limit.currency() != "USD" || self.limit.micros() == 0 {
            return Err(Error::Invalid("agent chat spend limit"));
        }
        Ok(())
    }
}
/// In-memory approval, granted by the composition root after the user accepted
/// the disclosure. Never deserialized and never created from model output.
#[derive(Clone, Debug)]
pub struct AgentChatConsent {
    scope: AgentChatScope,
}
impl AgentChatConsent {
    pub fn grant(scope: AgentChatScope) -> Result<Self, Error> {
        scope.validate()?;
        Ok(Self { scope })
    }
    pub fn scope(&self) -> &AgentChatScope {
        &self.scope
    }
}

/// Per-request and per-conversation bounds. Steps per turn belong to the turn loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentChatLimits {
    /// Output tokens one request may generate.
    pub max_output_tokens: u32,
    /// Bytes of the whole request body, history included.
    pub max_context_bytes: u64,
}
impl Default for AgentChatLimits {
    fn default() -> Self {
        Self {
            max_output_tokens: 4096,
            max_context_bytes: 262_144,
        }
    }
}
impl AgentChatLimits {
    fn validate(&self) -> Result<(), Error> {
        if !(1..=MAX_OUTPUT_TOKENS).contains(&self.max_output_tokens)
            || !(1..=MAX_CONTEXT_BYTES).contains(&self.max_context_bytes)
        {
            return Err(Error::Invalid("agent chat limits"));
        }
        Ok(())
    }
}

/// A tool the model may call; the schema comes from the owner's registry.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema object (`"type": "object"`).
    pub input_schema: Value,
}
impl ToolSpec {
    fn validate(&self) -> Result<(), Error> {
        let name_ok = (1..=64).contains(&self.name.len())
            && self
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
        let schema_ok = self.input_schema.get("type").and_then(Value::as_str) == Some("object");
        if !name_ok || !schema_ok {
            return Err(Error::Invalid("tool definition"));
        }
        Ok(())
    }
}

/// Arguments as the model produced them. Anything that is not a JSON object is
/// `Malformed` and never reaches a tool: the conversation answers it with an
/// error result the model can read.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum ToolInput {
    Valid(Value),
    Malformed,
}
impl fmt::Debug for ToolInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Valid(_) => f.write_str("Valid([input])"),
            Self::Malformed => f.write_str("Malformed"),
        }
    }
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: ToolInput,
}
impl fmt::Debug for ToolCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolCall")
            .field("id", &self.id)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}
/// The outcome of one tool call. Content is data for the model, never instructions.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: String,
    pub content: String,
    pub is_error: bool,
    /// Kinds of project data inside `content`; each must be inside the consent.
    pub kinds: BTreeSet<DataKind>,
}
impl fmt::Debug for ToolResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolResult")
            .field("call_id", &self.call_id)
            .field("is_error", &self.is_error)
            .finish_non_exhaustive()
    }
}

/// What the adapter that produced an assistant turn needs back, verbatim.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Replay {
    backend: String,
    items: Value,
}
impl Replay {
    pub(crate) fn new(backend: &str, items: Value) -> Self {
        Self {
            backend: backend.to_owned(),
            items,
        }
    }
    /// The recorded items, refused when another provider wrote them.
    pub(crate) fn items(&self, backend: &str) -> Result<&Value, Error> {
        if self.backend != backend {
            return Err(Error::Invalid(
                "conversation was recorded with another provider",
            ));
        }
        Ok(&self.items)
    }
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantTurn {
    pub text: String,
    pub calls: Vec<ToolCall>,
    replay: Replay,
}
impl AssistantTurn {
    pub(crate) fn new(text: String, calls: Vec<ToolCall>, replay: Replay) -> Self {
        Self {
            text,
            calls,
            replay,
        }
    }
    pub(crate) fn replay(&self) -> &Replay {
        &self.replay
    }
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    User(String),
    Assistant(AssistantTurn),
    ToolResults(Vec<ToolResult>),
}
impl fmt::Debug for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User(_) => f.write_str("User([text])"),
            Self::Assistant(turn) => write!(f, "Assistant({} calls)", turn.calls.len()),
            Self::ToolResults(results) => write!(f, "ToolResults({})", results.len()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Streamed output while a reply is generated. Tool calls are announced once their
/// arguments are complete, malformed ones included.
#[derive(Debug)]
pub enum ChatEvent<'a> {
    TextDelta(&'a str),
    ToolCall(&'a ToolCall),
}

/// Everything one request is built from.
pub struct ChatRequest<'a> {
    pub system: &'a str,
    pub tools: &'a [ToolSpec],
    pub messages: &'a [Message],
    pub max_output_tokens: u32,
}
/// An encoded request that has not been sent.
pub struct PreparedRequest {
    pub(crate) body: Vec<u8>,
}
impl PreparedRequest {
    pub fn len(&self) -> usize {
        self.body.len()
    }
    pub fn is_empty(&self) -> bool {
        self.body.is_empty()
    }
}
impl fmt::Debug for PreparedRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedRequest")
            .field("body_bytes", &self.body.len())
            .finish()
    }
}
/// A finished assistant message and what it cost, as the provider reported it.
pub struct ChatReply {
    pub assistant: AssistantTurn,
    pub usage: Option<Usage>,
}

/// One streamed request/response with a provider. Blocking and object safe; the
/// adapter never retries, never switches provider and observes cancellation
/// between stream events. `prepare` does no IO.
pub trait ChatModel: Send + Sync {
    fn config(&self) -> &ProviderConfig;
    /// Display name for the disclosure.
    fn provider(&self) -> &'static str;
    /// Provider-owned policy and documentation links in display order.
    fn links(&self) -> &'static [(&'static str, &'static str)];
    fn prepare(&self, request: &ChatRequest<'_>) -> Result<PreparedRequest, Error>;
    fn stream(
        &self,
        request: PreparedRequest,
        on_event: &mut dyn FnMut(ChatEvent<'_>),
        hooks: &dyn ExecutionHooks,
    ) -> Result<ChatReply, Error>;
}

/// What the app shows before the first request of a conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationDisclosure {
    pub provider: NfcText,
    pub model: NfcText,
    pub kinds: BTreeSet<DataKind>,
    pub limit: Money,
    pub max_context_bytes: u64,
    pub max_output_tokens: u32,
    pub links: Vec<(&'static str, NfcText)>,
}
pub fn disclosure(
    model: &dyn ChatModel,
    scope: &AgentChatScope,
    limits: &AgentChatLimits,
) -> Result<ConversationDisclosure, Error> {
    scope.validate()?;
    limits.validate()?;
    if scope.provider != *model.config() {
        return Err(Error::Unsupported);
    }
    if ledger::price(&scope.provider).is_none() {
        return Err(Error::Cost);
    }
    Ok(ConversationDisclosure {
        provider: model.provider().into(),
        model: scope.provider.model().clone(),
        kinds: scope.kinds.clone(),
        limit: scope.limit.clone(),
        max_context_bytes: limits.max_context_bytes,
        max_output_tokens: limits.max_output_tokens,
        links: model
            .links()
            .iter()
            .map(|(name, url)| (*name, NfcText::from(*url)))
            .collect(),
    })
}

/// One assistant message. When `needs_results`, answer with
/// [`Conversation::send_tool_results`] for exactly `calls`; `rejected` calls are
/// already answered with an error result the model will read.
#[derive(Clone, Debug)]
pub struct Reply {
    pub text: String,
    /// Valid calls to a listed tool, in the model's order.
    pub calls: Vec<ToolCall>,
    /// The error results the conversation wrote for malformed or unknown calls.
    pub rejected: Vec<ToolResult>,
    pub usage: Option<Usage>,
    /// Micro-USD settled so far for the whole conversation.
    pub spent_micros: u64,
}
impl Reply {
    pub fn needs_results(&self) -> bool {
        !self.calls.is_empty() || !self.rejected.is_empty()
    }
}

struct Awaiting {
    order: Vec<String>,
    rejected: BTreeMap<String, ToolResult>,
}

/// History, consent and ledger of one conversation. A failed request leaves the
/// history untouched, so the caller retries by calling the same method again;
/// each attempt reserves against the ledger on its own.
pub struct Conversation {
    model: Box<dyn ChatModel>,
    scope: AgentChatScope,
    price: ledger::Price,
    limits: AgentChatLimits,
    system: String,
    tools: Vec<ToolSpec>,
    history: Vec<Message>,
    ledger: Ledger,
    awaiting: Option<Awaiting>,
}
impl fmt::Debug for Conversation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Conversation")
            .field("scope", &self.scope)
            .field("messages", &self.history.len())
            .field("ledger", &self.ledger)
            .finish_non_exhaustive()
    }
}
impl Conversation {
    /// Refuses without a consent for exactly this provider and model, a model
    /// whose price is unknown (the limit could not be enforced) and invalid
    /// limits or tools. Nothing is sent here.
    pub fn new(
        model: Box<dyn ChatModel>,
        consent: Option<&AgentChatConsent>,
        system: impl Into<String>,
        tools: Vec<ToolSpec>,
        limits: AgentChatLimits,
    ) -> Result<Self, Error> {
        let scope = consent.ok_or(Error::ConsentRequired)?.scope().clone();
        scope.validate()?;
        if scope.provider != *model.config() {
            return Err(Error::ConsentRequired);
        }
        limits.validate()?;
        let names: BTreeSet<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
        if tools.len() > MAX_TOOLS || names.len() != tools.len() {
            return Err(Error::Invalid("tool definitions"));
        }
        for tool in &tools {
            tool.validate()?;
        }
        let price = ledger::price(&scope.provider).ok_or(Error::Cost)?;
        let ledger = Ledger::new(scope.limit.micros());
        Ok(Self {
            model,
            scope,
            price,
            limits,
            system: system.into(),
            tools,
            history: Vec::new(),
            ledger,
            awaiting: None,
        })
    }

    pub fn history(&self) -> &[Message] {
        &self.history
    }
    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }
    pub fn scope(&self) -> &AgentChatScope {
        &self.scope
    }

    /// Drop every message after the first `len` and forget the tool calls still owed
    /// results, so a caller that abandoned a turn (cancelled, step limit, failed request)
    /// can start the next one. The ledger keeps what the dropped requests cost.
    pub fn truncate_history(&mut self, len: usize) {
        self.history.truncate(len);
        self.awaiting = None;
    }

    /// Send the user's next message and stream the reply.
    pub fn send_user(
        &mut self,
        text: &str,
        on_event: &mut dyn FnMut(ChatEvent<'_>),
        hooks: &dyn ExecutionHooks,
    ) -> Result<Reply, Error> {
        if self.awaiting.is_some() {
            return Err(Error::Invalid("tool results are pending"));
        }
        self.step(Message::User(text.to_owned()), on_event, hooks)
    }

    /// Answer the calls of the last reply: one result per call in `Reply::calls`,
    /// none for `Reply::rejected` (those are already answered).
    pub fn send_tool_results(
        &mut self,
        results: &[ToolResult],
        on_event: &mut dyn FnMut(ChatEvent<'_>),
        hooks: &dyn ExecutionHooks,
    ) -> Result<Reply, Error> {
        let awaiting = self
            .awaiting
            .as_ref()
            .ok_or(Error::Invalid("no tool calls are pending"))?;
        let mut given: BTreeMap<&str, &ToolResult> = BTreeMap::new();
        for result in results {
            if given.insert(result.call_id.as_str(), result).is_some() {
                return Err(Error::Invalid("duplicate tool result"));
            }
        }
        let mut merged = Vec::with_capacity(awaiting.order.len());
        for id in &awaiting.order {
            let answer = given.remove(id.as_str());
            match (awaiting.rejected.get(id), answer) {
                (Some(rejected), None) => merged.push(rejected.clone()),
                (None, Some(result)) => merged.push(result.clone()),
                _ => return Err(Error::Invalid("tool results do not match the calls")),
            }
        }
        if !given.is_empty() {
            return Err(Error::Invalid("tool results do not match the calls"));
        }
        if merged
            .iter()
            .any(|result| !result.kinds.is_subset(&self.scope.kinds))
        {
            return Err(Error::ConsentRequired);
        }
        self.step(Message::ToolResults(merged), on_event, hooks)
    }

    fn step(
        &mut self,
        message: Message,
        on_event: &mut dyn FnMut(ChatEvent<'_>),
        hooks: &dyn ExecutionHooks,
    ) -> Result<Reply, Error> {
        if hooks.cancelled() {
            return Err(Error::Cancelled);
        }
        let mark = self.history.len();
        self.history.push(message);
        match self.exchange(on_event, hooks) {
            Ok(reply) => Ok(reply),
            Err(error) => {
                self.history.truncate(mark);
                Err(error)
            }
        }
    }

    /// Admission, reservation, the streamed request and settlement, in that order;
    /// nothing before the stream touches the network.
    fn exchange(
        &mut self,
        on_event: &mut dyn FnMut(ChatEvent<'_>),
        hooks: &dyn ExecutionHooks,
    ) -> Result<Reply, Error> {
        let prepared = self.model.prepare(&ChatRequest {
            system: &self.system,
            tools: &self.tools,
            messages: &self.history,
            max_output_tokens: self.limits.max_output_tokens,
        })?;
        let body_bytes = u64::try_from(prepared.len()).map_err(|_| Error::Payload)?;
        if body_bytes > self.limits.max_context_bytes {
            return Err(Error::Payload);
        }
        let upper = self
            .price
            .upper_bound(body_bytes, u64::from(self.limits.max_output_tokens))?;
        self.ledger.reserve(upper)?;
        let reply = match self.model.stream(prepared, on_event, hooks) {
            Ok(reply) => reply,
            Err(error) => {
                self.ledger.settle(if charged(&error) { upper } else { 0 });
                return Err(error);
            }
        };
        let actual = match reply.usage {
            Some(usage) => self.price.usage(usage).unwrap_or(upper),
            None => upper,
        };
        self.ledger.settle(actual);
        self.accept(reply)
    }

    fn accept(&mut self, reply: ChatReply) -> Result<Reply, Error> {
        let ChatReply { assistant, usage } = reply;
        let mut order = Vec::with_capacity(assistant.calls.len());
        let mut calls = Vec::new();
        let mut rejected = BTreeMap::new();
        for call in &assistant.calls {
            if order.contains(&call.id) {
                return Err(Error::Schema);
            }
            order.push(call.id.clone());
            let known = self.tools.iter().any(|tool| tool.name == call.name);
            let refusal = match &call.input {
                ToolInput::Malformed => Some(MALFORMED),
                ToolInput::Valid(_) if !known => Some(UNKNOWN_TOOL),
                ToolInput::Valid(_) => None,
            };
            match refusal {
                Some(content) => {
                    rejected.insert(
                        call.id.clone(),
                        ToolResult {
                            call_id: call.id.clone(),
                            content: content.to_owned(),
                            is_error: true,
                            kinds: BTreeSet::new(),
                        },
                    );
                }
                None => calls.push(call.clone()),
            }
        }
        let text = assistant.text.clone();
        let rejected_results: Vec<ToolResult> = order
            .iter()
            .filter_map(|id| rejected.get(id).cloned())
            .collect();
        self.history.push(Message::Assistant(assistant));
        self.awaiting = (!order.is_empty()).then_some(Awaiting { order, rejected });
        Ok(Reply {
            text,
            calls,
            rejected: rejected_results,
            usage,
            spent_micros: self.ledger.spent_micros(),
        })
    }
}

/// Whether a failed attempt is charged at its reservation. A request the provider
/// refused before generating costs nothing; one that may have generated (network
/// loss, cancellation, a broken or incomplete stream) is charged in full, because
/// its usage is unknown.
fn charged(error: &Error) -> bool {
    !matches!(
        error,
        Error::Auth
            | Error::Quota
            | Error::RateLimit { .. }
            | Error::Payload
            | Error::Invalid(_)
            | Error::Unsupported
            | Error::ModelUnavailable { .. }
            | Error::ConsentRequired
            | Error::Cost
    )
}
