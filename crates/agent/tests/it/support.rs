//! Builds scripted conversations, runners and turns for the behaviour tests.
use agent::{AgentEvent, CancelFlag, Turn, TurnLimits, TurnRequest, new_conversation, run_turn};
use ai::Money;
use ai::text::conversation::{
    AgentChatConsent, AgentChatLimits, AgentChatScope, Conversation, DataKind, Message,
    ScriptedChat, ScriptedReply, ScriptedStep, ToolCall, ToolInput,
};
use engine::Session;
use engine::cache::{ArtifactKind, CacheKey, KeyInputs};
use engine::jobs::{JobRunner, RunnerConfig, SystemClock};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

pub(crate) fn all_kinds() -> BTreeSet<DataKind> {
    [
        DataKind::ProjectSummary,
        DataKind::ClipNames,
        DataKind::TranscriptText,
    ]
    .into_iter()
    .collect()
}

pub(crate) fn conversation_with(
    steps: Vec<ScriptedStep>,
    kinds: BTreeSet<DataKind>,
    limit_micros: u64,
) -> Conversation {
    let consent = AgentChatConsent::grant(AgentChatScope {
        provider: ScriptedChat::provider_config().unwrap(),
        kinds,
        limit: Money::new("USD", limit_micros).unwrap(),
    })
    .unwrap();
    new_conversation(
        Box::new(ScriptedChat::new(steps).unwrap()),
        &consent,
        AgentChatLimits::default(),
    )
    .unwrap()
}

pub(crate) fn conversation(steps: Vec<ScriptedStep>) -> Conversation {
    conversation_with(steps, all_kinds(), 5_000_000)
}

pub(crate) fn runner() -> JobRunner {
    JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 16,
            retained: 16,
            network_attempts: 1,
            backoff: Duration::from_millis(1),
            progress_interval: Duration::from_millis(10),
        },
        engine::ByteBudget::new(512 << 20).unwrap(),
        Arc::new(SystemClock::default()),
    )
    .unwrap()
}

pub(crate) fn job_key() -> CacheKey {
    CacheKey::new(KeyInputs {
        kind: ArtifactKind::AsrChunk,
        source: [0; 32],
        provider: "agent",
        model: "test",
        version: "1",
        settings: b"{}",
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
    .unwrap()
}

pub(crate) fn call(id: &str, name: &str, input: Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        input: ToolInput::Valid(input),
    }
}

pub(crate) fn run_command(id: &str, command: &str, params: Value) -> ToolCall {
    call(
        id,
        "command_run",
        json!({"command": command, "params": params}),
    )
}

/// A reply that only calls tools.
pub(crate) fn calls(calls: Vec<ToolCall>) -> ScriptedStep {
    ScriptedStep::Reply(ScriptedReply {
        text: String::new(),
        calls,
    })
}

/// A reply that only talks, which ends the turn.
pub(crate) fn say(text: &str) -> ScriptedStep {
    ScriptedStep::Reply(ScriptedReply {
        text: text.into(),
        calls: Vec::new(),
    })
}

/// A reply built from the results of the previous tool calls.
pub(crate) fn then(
    build: impl FnOnce(Vec<Value>) -> Vec<ToolCall> + Send + 'static,
) -> ScriptedStep {
    ScriptedStep::Compose(Box::new(move |messages: &[Message]| ScriptedReply {
        text: String::new(),
        calls: build(last_results(messages)),
    }))
}

/// Tool results are JSON; the conversation's own refusals of malformed or unknown calls are text.
fn parse(content: &str) -> Value {
    serde_json::from_str(content).unwrap_or_else(|_| Value::String(content.into()))
}

/// The decoded results of the latest tool-result message, in call order.
pub(crate) fn last_results(messages: &[Message]) -> Vec<Value> {
    messages
        .iter()
        .rev()
        .find_map(|message| match message {
            Message::ToolResults(results) => {
                Some(results.iter().map(|r| parse(&r.content)).collect())
            }
            _ => None,
        })
        .unwrap()
}

/// Every tool result the conversation holds: decoded content and the error flag.
pub(crate) fn results(conversation: &Conversation) -> Vec<(Value, bool)> {
    conversation
        .history()
        .iter()
        .filter_map(|message| match message {
            Message::ToolResults(results) => Some(results),
            _ => None,
        })
        .flatten()
        .map(|result| (parse(&result.content), result.is_error))
        .collect()
}

pub(crate) fn drive(
    session: &Session,
    conversation: &mut Conversation,
    runner: &mut JobRunner,
    limits: TurnLimits,
    cancel: &CancelFlag,
    mut watch: impl FnMut(&AgentEvent),
) -> (Turn, Vec<AgentEvent>) {
    let mut events = Vec::new();
    let turn = run_turn(
        TurnRequest {
            session,
            prompt: "Please edit the project.",
            conversation,
            runner,
            limits,
            cancel: cancel.clone(),
        },
        &mut |event| {
            watch(&event);
            events.push(event);
        },
    )
    .unwrap();
    (turn, events)
}
