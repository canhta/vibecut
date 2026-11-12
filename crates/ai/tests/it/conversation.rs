#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Streamed tool-use conversations over both adapters. Every behaviour is a row
//! over the two providers; the scripted transport panics on an unexpected request,
//! so "refused before any network call" is proved by an empty script.
use ai::anthropic::{ANTHROPIC_VERSION, ENDPOINT as ANTHROPIC_ENDPOINT};
use ai::conversation::{
    AgentChatConsent, AgentChatLimits, AgentChatScope, AnthropicChat, ChatEvent, ChatModel,
    ChatRequest, Conversation, DataKind, Message, OpenAiChat, ToolInput, ToolResult, ToolSpec,
    Usage, default_model, disclosure,
};
use ai::openai::TRANSLATION_ENDPOINT as OPENAI_RESPONSES;
use ai::transport::ApiKey;
use ai::{Error, ExecutionHooks, Money, ProviderConfig, Stage};
use serde_json::{Value, json};
use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use transport::{HttpRequest, HttpStreamResponse, StreamTransport};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Provider {
    Anthropic,
    OpenAi,
}
const PROVIDERS: [Provider; 2] = [Provider::Anthropic, Provider::OpenAi];

impl Provider {
    fn dir(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
        }
    }
    fn model_id(self) -> &'static str {
        default_model(self.dir()).unwrap()
    }
    fn config(self, model: &str) -> ProviderConfig {
        match self {
            Self::Anthropic => AnthropicChat::provider_config(model).unwrap(),
            Self::OpenAi => OpenAiChat::provider_config(model).unwrap(),
        }
    }
    fn adapter(self, config: ProviderConfig, mock: &Arc<Mock>) -> Box<dyn ChatModel> {
        let key = ApiKey::new("test-key").unwrap();
        let transport: Arc<dyn StreamTransport> = mock.clone();
        match self {
            Self::Anthropic => Box::new(AnthropicChat::new(config, key, transport).unwrap()),
            Self::OpenAi => Box::new(OpenAiChat::new(config, key, transport).unwrap()),
        }
    }
    fn fixture(self, name: &str) -> Vec<u8> {
        fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/conversation")
                .join(self.dir())
                .join(name),
        )
        .unwrap()
    }
    fn endpoint(self) -> &'static str {
        match self {
            Self::Anthropic => ANTHROPIC_ENDPOINT,
            Self::OpenAi => OPENAI_RESPONSES,
        }
    }
    fn output_limit_field(self) -> &'static str {
        match self {
            Self::Anthropic => "max_tokens",
            Self::OpenAi => "max_output_tokens",
        }
    }
    /// The call ids in `tool.sse`, `parallel.sse` and `malformed.sse`.
    fn ids(self) -> (&'static str, [&'static str; 2], &'static str) {
        match self {
            Self::Anthropic => ("toolu_01", ["toolu_a", "toolu_b"], "toolu_bad"),
            Self::OpenAi => ("call_1", ["call_a", "call_b"], "call_bad"),
        }
    }
}

enum Script {
    Reply {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    Network,
}
impl Script {
    fn ok(body: Vec<u8>) -> Self {
        Self::Reply {
            status: 200,
            headers: Vec::new(),
            body,
        }
    }
    fn status(status: u16, body: &Value) -> Self {
        Self::Reply {
            status,
            headers: vec![("retry-after".into(), "7".into())],
            body: serde_json::to_vec(body).unwrap(),
        }
    }
}
#[derive(Clone)]
struct Recorded {
    url: String,
    headers: Vec<(String, String)>,
    body: Value,
    bytes: usize,
}
struct Mock {
    script: Mutex<VecDeque<Script>>,
    requests: Mutex<Vec<Recorded>>,
}
impl Mock {
    fn new(script: impl IntoIterator<Item = Script>) -> Arc<Self> {
        Arc::new(Self {
            script: Mutex::new(script.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
        })
    }
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
    fn request(&self, index: usize) -> Recorded {
        self.requests.lock().unwrap()[index].clone()
    }
}
impl StreamTransport for Mock {
    fn post_stream(&self, request: HttpRequest) -> Result<HttpStreamResponse, transport::Error> {
        self.requests.lock().unwrap().push(Recorded {
            url: request.url.clone(),
            headers: request.headers.clone(),
            body: serde_json::from_slice(&request.body).unwrap(),
            bytes: request.body.len(),
        });
        match self
            .script
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected network request")
        {
            Script::Reply {
                status,
                headers,
                body,
            } => Ok(HttpStreamResponse {
                status,
                headers,
                body: Box::new(Cursor::new(body)),
            }),
            Script::Network => Err(transport::Error::Network),
        }
    }
}

#[derive(Default)]
struct Hooks(AtomicBool);
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
    fn stage(&self, _: Stage) {}
}

fn tools() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "get_clip".into(),
        description: "Read one clip".into(),
        input_schema: json!({
            "type": "object",
            "properties": {"clip": {"type": "string"}},
            "required": ["clip"],
        }),
    }]
}
fn kinds(values: &[DataKind]) -> BTreeSet<DataKind> {
    values.iter().copied().collect()
}
fn scope(config: ProviderConfig, limit_micros: u64, granted: &[DataKind]) -> AgentChatScope {
    AgentChatScope {
        provider: config,
        kinds: kinds(granted),
        limit: Money::new("USD", limit_micros).unwrap(),
    }
}
fn chat(
    provider: Provider,
    mock: &Arc<Mock>,
    limit_micros: u64,
    limits: AgentChatLimits,
) -> Conversation {
    let config = provider.config(provider.model_id());
    let consent = AgentChatConsent::grant(scope(
        config.clone(),
        limit_micros,
        &[DataKind::ProjectSummary, DataKind::ClipNames],
    ))
    .unwrap();
    Conversation::new(
        provider.adapter(config, mock),
        Some(&consent),
        "You edit video.",
        tools(),
        limits,
    )
    .unwrap()
}
fn result(call_id: &str) -> ToolResult {
    ToolResult {
        call_id: call_id.into(),
        content: "{\"name\":\"intro\"}".into(),
        is_error: false,
        kinds: kinds(&[DataKind::ClipNames]),
    }
}
fn no_events(_: ChatEvent<'_>) {}

#[test]
fn streams_text_and_settles_the_reported_usage() {
    for provider in PROVIDERS {
        let mock = Mock::new([Script::ok(provider.fixture("text.sse"))]);
        let mut conversation = chat(provider, &mock, 1_000_000, AgentChatLimits::default());
        let mut seen = Vec::new();
        let reply = conversation
            .send_user(
                "hi",
                &mut |event: ChatEvent<'_>| {
                    if let ChatEvent::TextDelta(piece) = event {
                        seen.push(piece.to_owned());
                    }
                },
                &Hooks::default(),
            )
            .unwrap();
        assert_eq!(seen, ["Hello", " there"], "{provider:?}");
        assert_eq!(reply.text, "Hello there");
        assert!(!reply.needs_results());
        assert_eq!(
            reply.usage,
            Some(Usage {
                input_tokens: 100,
                output_tokens: 20
            })
        );
        // 100 * $2 + 20 * $10 per million tokens, in micro-USD.
        assert_eq!(reply.spent_micros, 400, "{provider:?}");
        assert_eq!(conversation.ledger().spent_micros(), 400);
        assert_eq!(conversation.history().len(), 2);

        let request = mock.request(0);
        assert_eq!(request.url, provider.endpoint());
        let (header, value) = match provider {
            Provider::Anthropic => ("x-api-key", "test-key".to_owned()),
            Provider::OpenAi => ("Authorization", "Bearer test-key".to_owned()),
        };
        assert!(request.headers.contains(&(header.into(), value)));
        assert!(
            request
                .headers
                .contains(&("Accept".into(), "text/event-stream".into()))
        );
        if provider == Provider::Anthropic {
            assert!(
                request
                    .headers
                    .contains(&("anthropic-version".into(), ANTHROPIC_VERSION.into()))
            );
        }
        assert_eq!(request.body["stream"], true);
        assert_eq!(request.body["model"], provider.model_id());
        assert_eq!(request.body[provider.output_limit_field()], 4096);
        assert_eq!(request.body["tools"][0]["name"], "get_clip");
    }
}

#[test]
fn tool_call_round_trip_replays_the_assistant_turn_and_answers_by_id() {
    for provider in PROVIDERS {
        let mock = Mock::new([
            Script::ok(provider.fixture("tool.sse")),
            Script::ok(provider.fixture("text.sse")),
        ]);
        let mut conversation = chat(provider, &mock, 1_000_000, AgentChatLimits::default());
        let mut seen = Vec::new();
        let reply = conversation
            .send_user(
                "what is in c1?",
                &mut |event: ChatEvent<'_>| {
                    if let ChatEvent::ToolCall(call) = event {
                        seen.push(call.name.clone());
                    }
                },
                &Hooks::default(),
            )
            .unwrap();
        let (id, _, _) = provider.ids();
        assert_eq!(seen, ["get_clip"]);
        assert_eq!(reply.text, "Let me look.");
        assert!(reply.needs_results() && reply.rejected.is_empty());
        assert_eq!(reply.calls.len(), 1);
        assert_eq!(reply.calls[0].id, id);
        assert_eq!(
            reply.calls[0].input,
            ToolInput::Valid(json!({"clip": "c1"}))
        );
        assert_eq!(reply.spent_micros, 740);
        // The tool results are owed before anything else is sent.
        assert!(matches!(
            conversation.send_user("again", &mut no_events, &Hooks::default()),
            Err(Error::Invalid(_))
        ));

        let reply = conversation
            .send_tool_results(&[result(id)], &mut no_events, &Hooks::default())
            .unwrap();
        assert_eq!(reply.text, "Hello there");
        assert!(!reply.needs_results());
        assert_eq!(reply.spent_micros, 740 + 400);

        let second = mock.request(1).body;
        match provider {
            Provider::Anthropic => {
                let messages = second["messages"].as_array().unwrap();
                assert_eq!(messages.len(), 3);
                let assistant = &messages[1]["content"];
                assert_eq!(assistant[0]["type"], "thinking");
                assert_eq!(assistant[0]["signature"], "sig-1");
                assert_eq!(assistant[1]["text"], "Let me look.");
                assert_eq!(assistant[2]["id"], id);
                assert_eq!(assistant[2]["input"], json!({"clip": "c1"}));
                let answer = &messages[2]["content"][0];
                assert_eq!(answer["type"], "tool_result");
                assert_eq!(answer["tool_use_id"], id);
                assert_eq!(answer["content"], "{\"name\":\"intro\"}");
                assert!(answer.get("is_error").is_none());
            }
            Provider::OpenAi => {
                let items = second["input"].as_array().unwrap();
                let types: Vec<&str> = items
                    .iter()
                    .map(|item| item["type"].as_str().unwrap_or("user"))
                    .collect();
                assert_eq!(
                    types,
                    [
                        "user",
                        "reasoning",
                        "message",
                        "function_call",
                        "function_call_output"
                    ]
                );
                assert_eq!(items[1]["encrypted_content"], "enc-1");
                assert_eq!(items[4]["call_id"], id);
                assert_eq!(items[4]["output"], "{\"name\":\"intro\"}");
                assert_eq!(second["store"], false);
            }
        }
    }
}

#[test]
fn parallel_calls_are_answered_together_and_an_incomplete_answer_refuses_before_the_network() {
    for provider in PROVIDERS {
        let mock = Mock::new([
            Script::ok(provider.fixture("parallel.sse")),
            Script::ok(provider.fixture("text.sse")),
        ]);
        let mut conversation = chat(provider, &mock, 1_000_000, AgentChatLimits::default());
        let reply = conversation
            .send_user("both clips", &mut no_events, &Hooks::default())
            .unwrap();
        let (_, [first, second], _) = provider.ids();
        let ids: Vec<&str> = reply.calls.iter().map(|call| call.id.as_str()).collect();
        assert_eq!(ids, [first, second]);

        for wrong in [
            vec![result(second)],
            vec![result(first), result(second), result("unrelated")],
            vec![result(first), result(first)],
        ] {
            assert!(matches!(
                conversation.send_tool_results(&wrong, &mut no_events, &Hooks::default()),
                Err(Error::Invalid(_))
            ));
        }
        assert_eq!(mock.count(), 1, "a wrong answer reached the network");
        assert_eq!(conversation.history().len(), 2);

        // Answers may come in any order; the request keeps the model's order.
        conversation
            .send_tool_results(
                &[result(second), result(first)],
                &mut no_events,
                &Hooks::default(),
            )
            .unwrap();
        let body = mock.request(1).body;
        let answered: Vec<String> = match provider {
            Provider::Anthropic => body["messages"][2]["content"]
                .as_array()
                .unwrap()
                .iter()
                .map(|block| block["tool_use_id"].as_str().unwrap().to_owned())
                .collect(),
            Provider::OpenAi => body["input"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| item["type"] == "function_call_output")
                .map(|item| item["call_id"].as_str().unwrap().to_owned())
                .collect(),
        };
        assert_eq!(answered, [first, second]);
    }
}

#[test]
fn refuses_without_a_matching_consent_before_any_network_call() {
    for provider in PROVIDERS {
        let mock = Mock::new([]);
        let limits = AgentChatLimits::default();
        let config = provider.config(provider.model_id());
        let new = |consent: Option<&AgentChatConsent>| {
            Conversation::new(
                provider.adapter(config.clone(), &mock),
                consent,
                "sys",
                tools(),
                limits,
            )
        };
        assert!(matches!(new(None), Err(Error::ConsentRequired)));

        // A consent for another model of the same provider does not carry over.
        let other =
            AgentChatConsent::grant(scope(provider.config("another-model"), 1_000, &[])).unwrap();
        assert!(matches!(new(Some(&other)), Err(Error::ConsentRequired)));

        // A consent cannot be granted without a positive USD limit.
        for limit in [
            Money::new("USD", 0).unwrap(),
            Money::new("EUR", 1_000).unwrap(),
        ] {
            let bad = AgentChatScope {
                provider: config.clone(),
                kinds: BTreeSet::new(),
                limit,
            };
            assert!(matches!(
                AgentChatConsent::grant(bad),
                Err(Error::Invalid(_))
            ));
        }

        // Data of a kind the user did not allow never leaves, even when a tool returned it.
        let mock = Mock::new([Script::ok(provider.fixture("tool.sse"))]);
        let mut conversation = chat(provider, &mock, 1_000_000, limits);
        let reply = conversation
            .send_user("go", &mut no_events, &Hooks::default())
            .unwrap();
        let mut transcript = result(&reply.calls[0].id);
        transcript.kinds = kinds(&[DataKind::TranscriptText]);
        assert!(matches!(
            conversation.send_tool_results(&[transcript], &mut no_events, &Hooks::default()),
            Err(Error::ConsentRequired)
        ));
        assert_eq!(mock.count(), 1);
        assert_eq!(conversation.history().len(), 2);
    }
}

#[test]
fn refuses_above_the_limit_and_charges_a_failed_attempt_on_retry() {
    for provider in PROVIDERS {
        // Over the limit: refused before any network call, nothing recorded or spent.
        let mock = Mock::new([]);
        let mut conversation = chat(provider, &mock, 100, AgentChatLimits::default());
        assert!(matches!(
            conversation.send_user("hi", &mut no_events, &Hooks::default()),
            Err(Error::Cost)
        ));
        assert!(conversation.history().is_empty());
        assert_eq!(conversation.ledger().spent_micros(), 0);

        // Over the context ceiling: refused the same way.
        let tiny = AgentChatLimits {
            max_output_tokens: 100,
            max_context_bytes: 64,
        };
        let mut conversation = chat(provider, &mock, 1_000_000, tiny);
        assert!(matches!(
            conversation.send_user("hi", &mut no_events, &Hooks::default()),
            Err(Error::Payload)
        ));
        assert_eq!(mock.count(), 0);

        // The upper bound of one attempt: every body byte as an input token plus the
        // output ceiling, at $2 and $10 per million tokens.
        let limits = AgentChatLimits {
            max_output_tokens: 100,
            ..AgentChatLimits::default()
        };
        let probe = Mock::new([Script::Network]);
        let mut conversation = chat(provider, &probe, 1_000_000, limits);
        assert!(
            conversation
                .send_user("hi", &mut no_events, &Hooks::default())
                .is_err()
        );
        let upper = probe.request(0).bytes as u64 * 2 + 100 * 10;
        assert_eq!(conversation.ledger().spent_micros(), upper);

        // A failed attempt is charged at its reservation, so the retry that would
        // pass the limit is refused without a second request.
        let mock = Mock::new([Script::Network]);
        let mut conversation = chat(provider, &mock, upper + upper / 2, limits);
        assert!(matches!(
            conversation.send_user("hi", &mut no_events, &Hooks::default()),
            Err(Error::Network)
        ));
        assert_eq!(conversation.ledger().spent_micros(), upper);
        assert!(conversation.history().is_empty());
        assert!(matches!(
            conversation.send_user("hi", &mut no_events, &Hooks::default()),
            Err(Error::Cost)
        ));
        assert_eq!(mock.count(), 1);
    }
}

#[test]
fn cancel_mid_stream_stops_charges_the_reservation_and_keeps_the_history() {
    for provider in PROVIDERS {
        let mock = Mock::new([Script::ok(provider.fixture("text.sse"))]);
        let mut conversation = chat(provider, &mock, 1_000_000, AgentChatLimits::default());
        let hooks = Hooks::default();
        let mut deltas = 0;
        let error = conversation
            .send_user(
                "hi",
                &mut |event: ChatEvent<'_>| {
                    if matches!(event, ChatEvent::TextDelta(_)) {
                        deltas += 1;
                        hooks.0.store(true, Ordering::Relaxed);
                    }
                },
                &hooks,
            )
            .unwrap_err();
        assert!(matches!(error, Error::Cancelled), "{provider:?}");
        assert_eq!(deltas, 1, "the stream kept being read after the cancel");
        assert!(conversation.history().is_empty());
        let upper = mock.request(0).bytes as u64 * 2 + 4096 * 10;
        assert_eq!(conversation.ledger().spent_micros(), upper);

        // Cancelled before the request: nothing is reserved or sent.
        let mut conversation = chat(provider, &mock, 1_000_000, AgentChatLimits::default());
        assert!(matches!(
            conversation.send_user("hi", &mut no_events, &hooks),
            Err(Error::Cancelled)
        ));
        assert_eq!(mock.count(), 1);
        assert_eq!(conversation.ledger().spent_micros(), 0);
    }
}

#[test]
fn malformed_or_unknown_tool_calls_are_answered_with_a_tool_error() {
    for provider in PROVIDERS {
        let (_, _, bad) = provider.ids();
        let mock = Mock::new([
            Script::ok(provider.fixture("malformed.sse")),
            Script::ok(provider.fixture("text.sse")),
        ]);
        let mut conversation = chat(provider, &mock, 1_000_000, AgentChatLimits::default());
        let mut announced = Vec::new();
        let reply = conversation
            .send_user(
                "go",
                &mut |event: ChatEvent<'_>| {
                    if let ChatEvent::ToolCall(call) = event {
                        announced.push(call.input == ToolInput::Malformed);
                    }
                },
                &Hooks::default(),
            )
            .unwrap();
        assert_eq!(announced, [true]);
        assert!(reply.calls.is_empty() && reply.needs_results());
        assert_eq!(reply.rejected.len(), 1);
        assert!(reply.rejected[0].is_error);
        assert!(
            reply.rejected[0]
                .content
                .contains("not a valid JSON object")
        );

        // The caller owes nothing for it; the model reads the error and carries on.
        let reply = conversation
            .send_tool_results(&[], &mut no_events, &Hooks::default())
            .unwrap();
        assert_eq!(reply.text, "Hello there");
        let body = mock.request(1).body;
        match provider {
            Provider::Anthropic => {
                let answer = &body["messages"][2]["content"][0];
                assert_eq!(answer["tool_use_id"], bad);
                assert_eq!(answer["is_error"], true);
                assert!(
                    answer["content"]
                        .as_str()
                        .unwrap()
                        .contains("not a valid JSON")
                );
                assert_eq!(body["messages"][1]["content"][0]["input"], json!({}));
            }
            Provider::OpenAi => {
                let items = body["input"].as_array().unwrap();
                assert_eq!(items[1]["arguments"], "{}");
                assert_eq!(items[2]["call_id"], bad);
                assert!(
                    items[2]["output"]
                        .as_str()
                        .unwrap()
                        .starts_with("error: The tool arguments")
                );
            }
        }

        // Valid JSON for a tool that was not offered is refused the same way, and the
        // caller cannot answer for a call the conversation already answered.
        let renamed = String::from_utf8(provider.fixture("tool.sse"))
            .unwrap()
            .replace("get_clip", "delete_everything");
        let mock = Mock::new([Script::ok(renamed.into_bytes())]);
        let mut conversation = chat(provider, &mock, 1_000_000, AgentChatLimits::default());
        let reply = conversation
            .send_user("go", &mut no_events, &Hooks::default())
            .unwrap();
        assert!(reply.calls.is_empty());
        assert!(reply.rejected[0].content.contains("No such tool"));
        assert!(matches!(
            conversation.send_tool_results(
                &[result(&reply.rejected[0].call_id)],
                &mut no_events,
                &Hooks::default()
            ),
            Err(Error::Invalid(_))
        ));
        assert_eq!(mock.count(), 1);
    }
}

fn events(items: &[Value]) -> Vec<u8> {
    items
        .iter()
        .map(|item| format!("data: {item}\n\n"))
        .collect::<String>()
        .into_bytes()
}
/// A text reply that ends with `finish` events, or is cut off when there are none.
fn anthropic_text(finish: &[Value]) -> Script {
    let mut stream = vec![
        json!({"type":"message_start","message":{"usage":{"input_tokens":10,"output_tokens":1}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"x"}}),
        json!({"type":"content_block_stop","index":0}),
    ];
    stream.extend_from_slice(finish);
    Script::ok(events(&stream))
}
fn anthropic_stop(reason: &str) -> Vec<Value> {
    vec![
        json!({"type":"message_delta","delta":{"stop_reason":reason},"usage":{"output_tokens":5}}),
        json!({"type":"message_stop"}),
    ]
}
fn openai_stream(tail: &[Value]) -> Script {
    let mut stream =
        vec![json!({"type":"response.output_text.delta","output_index":0,"delta":"x"})];
    stream.extend_from_slice(tail);
    Script::ok(events(&stream))
}

type Matcher = fn(&Error) -> bool;
/// (label, provider, scripted reply, expected error, charged at the reservation)
type Row = (&'static str, Provider, Script, Matcher, bool);
fn row(
    label: &'static str,
    provider: Provider,
    script: Script,
    expected: Matcher,
    charged: bool,
) -> Row {
    (label, provider, script, expected, charged)
}
fn failures() -> Vec<Row> {
    let long_line = || {
        let mut body = b"data: ".to_vec();
        body.extend(std::iter::repeat_n(b'x', 300_000));
        Script::ok(body)
    };
    let anthropic_error = |kind: &str| json!({"type":"error","error":{"type":kind,"message":"m"}});
    let openai_error = |code: &str| json!({"error":{"code":code,"message":"m"}});
    vec![
        row(
            "anthropic rate limit",
            Provider::Anthropic,
            Script::status(429, &anthropic_error("rate_limit_error")),
            |e| {
                matches!(
                    e,
                    Error::RateLimit {
                        retry_after_seconds: Some(7)
                    }
                )
            },
            false,
        ),
        row(
            "anthropic auth",
            Provider::Anthropic,
            Script::status(401, &anthropic_error("authentication_error")),
            |e| matches!(e, Error::Auth),
            false,
        ),
        row(
            "anthropic unknown model",
            Provider::Anthropic,
            Script::status(404, &anthropic_error("not_found_error")),
            |e| matches!(e, Error::ModelUnavailable { .. }),
            false,
        ),
        row(
            "anthropic error event",
            Provider::Anthropic,
            Script::ok(events(&[anthropic_error("overloaded_error")])),
            |e| matches!(e, Error::Server),
            true,
        ),
        row(
            "anthropic refusal",
            Provider::Anthropic,
            anthropic_text(&anthropic_stop("refusal")),
            |e| matches!(e, Error::Refused),
            true,
        ),
        row(
            "anthropic max tokens",
            Provider::Anthropic,
            anthropic_text(&anthropic_stop("max_tokens")),
            |e| matches!(e, Error::Incomplete),
            true,
        ),
        row(
            "anthropic cut stream",
            Provider::Anthropic,
            anthropic_text(&[]),
            |e| matches!(e, Error::Incomplete),
            true,
        ),
        row(
            "anthropic runaway line",
            Provider::Anthropic,
            long_line(),
            |e| matches!(e, Error::Payload),
            false,
        ),
        row(
            "openai rate limit",
            Provider::OpenAi,
            Script::status(429, &openai_error("rate_limit_exceeded")),
            |e| {
                matches!(
                    e,
                    Error::RateLimit {
                        retry_after_seconds: Some(7)
                    }
                )
            },
            false,
        ),
        row(
            "openai auth",
            Provider::OpenAi,
            Script::status(401, &openai_error("invalid_api_key")),
            |e| matches!(e, Error::Auth),
            false,
        ),
        row(
            "openai unknown model",
            Provider::OpenAi,
            Script::status(404, &openai_error("model_not_found")),
            |e| matches!(e, Error::ModelUnavailable { .. }),
            false,
        ),
        row(
            "openai failed",
            Provider::OpenAi,
            openai_stream(&[
                json!({"type":"response.failed","response":{"error":{"code":"server_error","message":"m"}}}),
            ]),
            |e| matches!(e, Error::Server),
            true,
        ),
        row(
            "openai incomplete",
            Provider::OpenAi,
            openai_stream(&[
                json!({"type":"response.incomplete","response":{"status":"incomplete"}}),
            ]),
            |e| matches!(e, Error::Incomplete),
            true,
        ),
        row(
            "openai refusal",
            Provider::OpenAi,
            openai_stream(&[json!({"type":"response.refusal.delta","delta":"no"})]),
            |e| matches!(e, Error::Refused),
            true,
        ),
        row(
            "openai cut stream",
            Provider::OpenAi,
            openai_stream(&[]),
            |e| matches!(e, Error::Incomplete),
            true,
        ),
        row(
            "openai runaway line",
            Provider::OpenAi,
            long_line(),
            |e| matches!(e, Error::Payload),
            false,
        ),
    ]
}

#[test]
fn provider_failures_map_to_typed_errors_and_leave_the_history_alone() {
    for (label, provider, script, expected, charged) in failures() {
        let mock = Mock::new([script]);
        let mut conversation = chat(provider, &mock, 1_000_000, AgentChatLimits::default());
        let error = conversation
            .send_user("hi", &mut no_events, &Hooks::default())
            .unwrap_err();
        assert!(expected(&error), "{label}: {error:?}");
        assert!(conversation.history().is_empty(), "{label}");
        assert_eq!(conversation.ledger().spent_micros() > 0, charged, "{label}");
        assert!(
            !format!("{error:?}").contains("\"m\""),
            "{label}: provider text leaked"
        );
    }
}

#[test]
fn history_recorded_by_one_provider_is_refused_by_the_other() {
    for (from, to) in [
        (Provider::Anthropic, Provider::OpenAi),
        (Provider::OpenAi, Provider::Anthropic),
    ] {
        let mock = Mock::new([Script::ok(from.fixture("tool.sse"))]);
        let mut conversation = chat(from, &mock, 1_000_000, AgentChatLimits::default());
        conversation
            .send_user("go", &mut no_events, &Hooks::default())
            .unwrap();
        let history: Vec<Message> = conversation.history().to_vec();
        let other = to.adapter(to.config(to.model_id()), &Mock::new([]));
        let error = other
            .prepare(&ChatRequest {
                system: "sys",
                tools: &tools(),
                messages: &history,
                max_output_tokens: 100,
            })
            .unwrap_err();
        assert!(matches!(error, Error::Invalid(_)), "{from:?} -> {to:?}");
    }
}

#[test]
fn disclosure_names_what_leaves_and_unpriced_models_cannot_hold_a_limit() {
    for provider in PROVIDERS {
        let mock = Mock::new([]);
        let config = provider.config(provider.model_id());
        let granted = scope(config.clone(), 5_000_000, &[DataKind::TranscriptText]);
        let model = provider.adapter(config, &mock);
        let shown = disclosure(model.as_ref(), &granted, &AgentChatLimits::default()).unwrap();
        assert_eq!(
            shown.provider.as_str(),
            match provider {
                Provider::Anthropic => "Anthropic",
                Provider::OpenAi => "OpenAI",
            }
        );
        assert_eq!(shown.model.as_str(), provider.model_id());
        assert_eq!(shown.kinds, granted.kinds);
        assert_eq!(shown.limit, granted.limit);
        assert_eq!(shown.max_context_bytes, 262_144);
        assert!(
            shown
                .links
                .iter()
                .any(|(name, url)| *name == "policy" && url.as_str().starts_with("https://"))
        );

        // A scope for a different model than the adapter's is not what is sent.
        let other = scope(provider.config("another-model"), 5_000_000, &[]);
        assert!(matches!(
            disclosure(model.as_ref(), &other, &AgentChatLimits::default()),
            Err(Error::Unsupported)
        ));

        // No registered price: the limit could not be enforced, so no conversation.
        let unpriced = provider.config("future-model");
        let model = provider.adapter(unpriced.clone(), &mock);
        let granted = scope(unpriced, 5_000_000, &[]);
        assert!(matches!(
            disclosure(model.as_ref(), &granted, &AgentChatLimits::default()),
            Err(Error::Cost)
        ));
        let consent = AgentChatConsent::grant(granted).unwrap();
        assert!(matches!(
            Conversation::new(
                model,
                Some(&consent),
                "sys",
                tools(),
                AgentChatLimits::default()
            ),
            Err(Error::Cost)
        ));
    }
    assert_eq!(default_model("anthropic"), Some("claude-sonnet-5-5"));
    assert_eq!(default_model("openai"), Some("gpt-6.1-sol"));
    assert_eq!(default_model("elevenlabs"), None);
}
