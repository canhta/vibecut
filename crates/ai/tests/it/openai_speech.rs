#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
use ai::openai::{
    OpenAiSpeechSynthesizer, SPEECH_ENDPOINT, SPEECH_LANGUAGES, SPEECH_SAMPLE_RATE, SPEECH_VOICES,
};
use ai::speech::{
    SpeechFormat, SpeechLanguage, SpeechOutput, SpeechRequest, SpeechSynthesizer, SynthesisConsent,
    VoiceCategory, prepare_synthesis, synthesize,
};
use ai::transport::{ApiKey, HttpRequest, HttpResponse, HttpTransport, UreqTransport};
use ai::{Error, ExecutionHooks, Money, ProviderConfig, Stage};
use base64::Engine as _;
use captions::LanguageTag;
use serde_json::Value;
use std::collections::VecDeque;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

struct NoHooks;
impl ExecutionHooks for NoHooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn stage(&self, _: Stage) {}
}

struct Hooks(Arc<AtomicBool>);
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    fn stage(&self, _: Stage) {}
}

enum Reply {
    Response {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    Failure(Error),
}

struct FakeTransport {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<HttpRequest>>,
}
impl FakeTransport {
    fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            requests: Mutex::new(Vec::new()),
        }
    }
    fn calls(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
    fn ok(body: Vec<u8>) -> Reply {
        Reply::Response {
            status: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            body,
        }
    }
}
impl HttpTransport for FakeTransport {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
        self.requests.lock().unwrap().push(request);
        match self.replies.lock().unwrap().pop_front() {
            Some(Reply::Response {
                status,
                headers,
                body,
            }) => Ok(HttpResponse {
                status,
                headers,
                body,
            }),
            Some(Reply::Failure(error)) => Err(error),
            None => Err(Error::Network),
        }
    }
}

/// Sets the shared cancellation flag while the provider call is in flight, so the
/// adapter's post-IO checkpoint can be exercised deterministically.
struct CancelTransport {
    flag: Arc<AtomicBool>,
    reply: Mutex<Option<Reply>>,
}
impl HttpTransport for CancelTransport {
    fn send(&self, _: HttpRequest) -> Result<HttpResponse, Error> {
        self.flag.store(true, Ordering::SeqCst);
        match self.reply.lock().unwrap().take() {
            Some(Reply::Response {
                status,
                headers,
                body,
            }) => Ok(HttpResponse {
                status,
                headers,
                body,
            }),
            Some(Reply::Failure(error)) => Err(error),
            None => Err(Error::Network),
        }
    }
}

fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/openai-speech")
            .join(name),
    )
    .unwrap()
}

fn config() -> ProviderConfig {
    OpenAiSpeechSynthesizer::provider_config("gpt-audio-1.5").unwrap()
}

fn request_with(
    text: &str,
    voice: &str,
    language: &str,
    script: &str,
    duration_micros: u64,
) -> SpeechRequest {
    SpeechRequest::new(
        config(),
        text.into(),
        voice.into(),
        SpeechLanguage {
            language: LanguageTag::parse(language).unwrap(),
            script: script.into(),
        },
        SpeechOutput {
            format: SpeechFormat::WavPcm16,
            sample_rate: SPEECH_SAMPLE_RATE,
        },
        duration_micros,
    )
    .unwrap()
}

fn request(text: &str) -> SpeechRequest {
    request_with(text, "marin", "vi", "Latn", 1_000_000)
}

const RECORDED_TEXT: &str = "Xin chào, đây là một câu kiểm tra giọng nói.";

fn recorded_request() -> SpeechRequest {
    request_with(RECORDED_TEXT, "marin", "vi", "Latn", 6_000_000)
}

fn adapter(fake: Arc<dyn HttpTransport>) -> OpenAiSpeechSynthesizer {
    OpenAiSpeechSynthesizer::new(config(), ApiKey::new("test-key-not-real").unwrap(), fake).unwrap()
}

fn consent(
    synth: &OpenAiSpeechSynthesizer,
    request: &SpeechRequest,
    budget: u64,
) -> Result<SynthesisConsent, Error> {
    SynthesisConsent::grant(
        request,
        &synth.capabilities(request),
        synth.estimate(request),
        Money::new("USD", budget).unwrap(),
    )
}

fn header<'a>(request: &'a HttpRequest, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|(header, _)| header.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn response_with_wav(wav: &[u8]) -> Vec<u8> {
    let mut body: Value = serde_json::from_slice(&fixture("success.json")).unwrap();
    body["choices"][0]["message"]["audio"]["data"] =
        base64::engine::general_purpose::STANDARD.encode(wav).into();
    serde_json::to_vec(&body).unwrap()
}

fn run_recorded_response(body: Vec<u8>) -> Result<ai::speech::SpeechAudio, Error> {
    let fake = Arc::new(FakeTransport::new(vec![FakeTransport::ok(body)]));
    let synth = adapter(fake);
    let request = recorded_request();
    let approved = consent(&synth, &request, 1_000_000).unwrap();
    synthesize(&synth, &request, &approved, &NoHooks)
}

#[test]
fn success_replays_recorded_fixture_and_exact_transcript() {
    let fake = Arc::new(FakeTransport::new(vec![FakeTransport::ok(fixture(
        "success.json",
    ))]));
    let synth = adapter(fake.clone());
    let request = recorded_request();
    let approved = consent(&synth, &request, 1_000_000).unwrap();
    let audio = synthesize(&synth, &request, &approved, &NoHooks).unwrap();

    assert_eq!(audio.samples.len(), 68_400);
    assert_eq!(audio.sample_rate, SPEECH_SAMPLE_RATE);
    assert_eq!(audio.duration_micros, 2_850_000);
    assert_eq!(audio.transcript.as_ref(), Some(request.text()));
    assert_eq!(audio.usage.input_tokens, Some(62));
    assert_eq!(audio.usage.output_tokens, Some(81));
    assert_eq!(audio.usage.actual_cost.as_ref().unwrap().micros(), 4_043);
    assert_eq!(audio.usage.actual_cost.as_ref().unwrap().currency(), "USD");
    assert_eq!(audio.fingerprint, synth.capabilities(&request).fingerprint);

    let requests = fake.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let outbound = &requests[0];
    let expected: Value = serde_json::from_slice(&fixture("request.json")).unwrap();
    assert_eq!(outbound.method, expected["method"].as_str().unwrap());
    assert_eq!(outbound.url, expected["url"].as_str().unwrap());
    assert_eq!(
        header(outbound, "authorization"),
        Some("Bearer test-key-not-real")
    );
    assert_eq!(header(outbound, "content-type"), Some("application/json"));
    assert!(!format!("{outbound:?}").contains("test-key-not-real"));
    let body: Value = serde_json::from_slice(&outbound.body).unwrap();
    assert_eq!(body["model"], expected["model"]);
    assert_eq!(body["modalities"], expected["modalities"]);
    assert_eq!(body["audio"], expected["audio"]);
    assert_eq!(body["store"], expected["store"]);
    assert_eq!(
        body["max_completion_tokens"],
        expected["max_completion_tokens"]
    );
    assert_eq!(body["messages"], expected["messages"]);
}

#[test]
fn nfd_input_is_normalized_before_transcript_comparison_and_wire_payload() {
    let fake = Arc::new(FakeTransport::new(vec![FakeTransport::ok(fixture(
        "success.json",
    ))]));
    let synth = adapter(fake.clone());
    let decomposed = "Xin cha\u{0300}o, đây là một câu kiểm tra giọng nói.";
    let request = request_with(decomposed, "marin", "vi", "Latn", 6_000_000);
    assert_eq!(request.text().as_str(), RECORDED_TEXT);
    let approved = consent(&synth, &request, 1_000_000).unwrap();
    let audio = synthesize(&synth, &request, &approved, &NoHooks).unwrap();
    assert_eq!(audio.transcript.as_ref(), Some(request.text()));
    let requests = fake.requests.lock().unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["messages"][1]["content"], RECORDED_TEXT);
}

#[test]
fn language_voice_and_constructor_boundaries_fail_before_network() {
    let fake = Arc::new(FakeTransport::new(Vec::new()));
    let synth = adapter(fake.clone());
    let valid = request("Xin chào");
    let approved = consent(&synth, &valid, 1_000_000).unwrap();

    for (language, script) in [("fr", "Latn"), ("zh", "Hant"), ("vi", "Hans")] {
        let unsupported = request_with("text", "marin", language, script, 1_000_000);
        assert!(matches!(
            synth.capabilities(&unsupported).check(&unsupported),
            Err(Error::UnsupportedLanguage)
        ));
        assert!(matches!(
            prepare_synthesis(&synth, &unsupported, &approved),
            Err(Error::UnsupportedLanguage)
        ));
    }
    let custom_voice = request_with("text", "voice_123", "vi", "Latn", 1_000_000);
    assert!(matches!(
        synth.capabilities(&custom_voice).check(&custom_voice),
        Err(Error::UnsupportedVoice)
    ));
    assert!(matches!(
        prepare_synthesis(&synth, &custom_voice, &approved),
        Err(Error::UnsupportedVoice)
    ));
    assert_eq!(fake.calls(), 0);
}

#[test]
fn transcript_mismatch_missing_transcript_and_broken_wav_are_rejected() {
    let cases = [
        ("transcript-mismatch.json", Error::OutputRejected),
        ("missing-transcript.json", Error::OutputRejected),
        ("error-truncated-wav.json", Error::OutputRejected),
        ("error-wrong-rate.json", Error::OutputRejected),
        ("error-odd-data.json", Error::OutputRejected),
        ("error-malformed.json", Error::Schema),
    ];
    for (name, expected) in cases {
        let fake = Arc::new(FakeTransport::new(vec![FakeTransport::ok(fixture(name))]));
        let synth = adapter(fake.clone());
        let request = request("Xin chào");
        let approved = consent(&synth, &request, 1_000_000).unwrap();
        let error = synthesize(&synth, &request, &approved, &NoHooks).unwrap_err();
        assert_eq!(
            std::mem::discriminant(&error),
            std::mem::discriminant(&expected),
            "fixture {name}"
        );
        assert!(!error.to_string().contains("Xin"));
        assert_eq!(fake.calls(), 1, "fixture {name}");
    }

    let fake = Arc::new(FakeTransport::new(vec![FakeTransport::ok(fixture(
        "success.json",
    ))]));
    let synth = adapter(fake.clone());
    let short = request_with("Xin chào", "marin", "vi", "Latn", 50_000);
    let approved = consent(&synth, &short, 1_000_000).unwrap();
    assert!(matches!(
        synthesize(&synth, &short, &approved, &NoHooks),
        Err(Error::OutputRejected)
    ));
    assert_eq!(fake.calls(), 1);
}

#[test]
fn payload_and_cost_bounds_refuse_before_network() {
    let fake = Arc::new(FakeTransport::new(Vec::new()));
    let synth = adapter(fake.clone());

    let within = request("Xin chào");
    match synth.estimate(&within) {
        ai::speech::SynthesisEstimate::UpperBound(cost) => {
            assert_eq!(cost.micros(), 8_535);
            assert_eq!(cost.currency(), "USD");
        }
        other => panic!("unexpected estimate: {other:?}"),
    }
    assert!(matches!(consent(&synth, &within, 8_534), Err(Error::Cost)));
    assert_eq!(fake.calls(), 0);

    let huge_text = "a".repeat(ai::speech::MAX_SPEECH_TEXT_BYTES);
    // One grapheme can contain many byte-BPE tokens; its quote must cover bytes.
    let combining_text = format!("q{}", "\u{0305}".repeat(10_000));
    for (text, duration_micros, minimum_quote, refused_budget) in [
        ("Xin chào", 1_000_000_000, 0, None),
        (huge_text.as_str(), 1_000_000, 0, None),
        (combining_text.as_str(), 1_000_000, 50_003, Some(8_003)),
        (combining_text.as_str(), 6_000_000, 50_003, Some(14_403)),
    ] {
        let request = request_with(text, "marin", "vi", "Latn", duration_micros);
        let quote = match synth.estimate(&request) {
            ai::speech::SynthesisEstimate::UpperBound(quote) => quote,
            other => panic!("unexpected estimate: {other:?}"),
        };
        assert!(quote.micros() >= minimum_quote);
        if let Some(budget) = refused_budget {
            assert!(matches!(
                consent(&synth, &request, budget),
                Err(Error::Cost)
            ));
        }
        let approved = consent(&synth, &request, quote.micros()).unwrap();
        assert!(matches!(
            synthesize(&synth, &request, &approved, &NoHooks),
            Err(Error::Payload)
        ));
        assert_eq!(fake.calls(), 0);
    }
}

#[test]
fn completion_status_must_be_successful() {
    for (reason, expected) in [
        (Some("length"), Error::OutputRejected),
        (Some("content_filter"), Error::ContentPolicy),
        (Some("tool_calls"), Error::OutputRejected),
        (Some("unknown"), Error::OutputRejected),
        (None, Error::OutputRejected),
    ] {
        let mut body: Value = serde_json::from_slice(&fixture("success.json")).unwrap();
        match reason {
            Some(reason) => body["choices"][0]["finish_reason"] = reason.into(),
            None => {
                body["choices"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("finish_reason");
            }
        }
        let fake = Arc::new(FakeTransport::new(vec![FakeTransport::ok(
            serde_json::to_vec(&body).unwrap(),
        )]));
        let synth = adapter(fake.clone());
        let request = recorded_request();
        let approved = consent(&synth, &request, 1_000_000).unwrap();
        let error = synthesize(&synth, &request, &approved, &NoHooks).unwrap_err();
        assert_eq!(
            std::mem::discriminant(&error),
            std::mem::discriminant(&expected),
            "finish_reason {reason:?}"
        );
        assert_eq!(fake.calls(), 1, "finish_reason {reason:?}");
    }
}

#[test]
fn finite_riff_boundaries_are_validated_and_sentinel_is_preserved() {
    let original = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/openai-speech/success.wav"),
    )
    .unwrap();

    let len = original.len();
    let mut finite = original.clone();
    finite[4..8].copy_from_slice(&u32::try_from(len - 8).unwrap().to_le_bytes());
    finite[40..44].copy_from_slice(&u32::try_from(len - 44).unwrap().to_le_bytes());
    assert!(run_recorded_response(response_with_wav(&finite)).is_ok());

    // A finite outer boundary still admits the observed 0xFFFFFFFF inner data
    // length and parses it inside the declared boundary.
    let mut finite_sentinel_data = original.clone();
    finite_sentinel_data[4..8].copy_from_slice(&u32::try_from(len - 8).unwrap().to_le_bytes());
    assert!(run_recorded_response(response_with_wav(&finite_sentinel_data)).is_ok());

    // Reviewer reproduction: a finite RIFF size larger than the received body.
    let mut truncated = original.clone();
    truncated[4..8].copy_from_slice(&u32::try_from(len + 100).unwrap().to_le_bytes());
    assert!(matches!(
        run_recorded_response(response_with_wav(&truncated)),
        Err(Error::OutputRejected)
    ));

    // Bytes outside a smaller finite RIFF boundary are trailing data.
    let mut trailing = original.clone();
    trailing[4..8].copy_from_slice(&u32::try_from(len - 8 - 4).unwrap().to_le_bytes());
    assert!(matches!(
        run_recorded_response(response_with_wav(&trailing)),
        Err(Error::OutputRejected)
    ));
}

#[test]
fn provider_failures_are_typed_without_response_bodies_and_do_not_retry() {
    let fake = Arc::new(FakeTransport::new(vec![
        Reply::Response {
            status: 401,
            headers: Vec::new(),
            body: fixture("error-auth.json"),
        },
        Reply::Response {
            status: 429,
            headers: Vec::new(),
            body: fixture("error-quota.json"),
        },
        Reply::Response {
            status: 429,
            headers: vec![("Retry-After".into(), "7".into())],
            body: fixture("error-rate-limit.json"),
        },
        Reply::Response {
            status: 500,
            headers: Vec::new(),
            body: fixture("error-server.json"),
        },
        Reply::Response {
            status: 400,
            headers: Vec::new(),
            body: fixture("error-content-policy.json"),
        },
        Reply::Response {
            status: 400,
            headers: Vec::new(),
            body: fixture("error-invalid-voice.json"),
        },
        Reply::Response {
            status: 413,
            headers: Vec::new(),
            body: fixture("error-payload.json"),
        },
        Reply::Response {
            status: 302,
            headers: Vec::new(),
            body: Vec::new(),
        },
        Reply::Failure(Error::Network),
    ]));
    let synth = adapter(fake.clone());
    let request = request("private text");
    let approved = consent(&synth, &request, 1_000_000).unwrap();

    let cases: Vec<Error> = (0..9)
        .map(|_| synthesize(&synth, &request, &approved, &NoHooks).expect_err("fixture must fail"))
        .collect();
    assert!(matches!(cases[0], Error::Auth));
    assert!(matches!(cases[1], Error::Quota));
    assert!(matches!(
        cases[2],
        Error::RateLimit {
            retry_after_seconds: Some(7)
        }
    ));
    assert!(matches!(cases[3], Error::Server));
    assert!(matches!(cases[4], Error::ContentPolicy));
    assert!(matches!(cases[5], Error::UnsupportedVoice));
    assert!(matches!(cases[6], Error::Payload));
    assert!(matches!(cases[7], Error::Invalid(_)));
    assert!(matches!(cases[8], Error::Network));
    for error in &cases {
        let display = error.to_string();
        assert!(!display.contains("private"));
        assert!(!display.contains("sk-secret"));
        assert!(!display.contains("billing details"));
        assert!(!display.contains("Rate limit"));
    }
    assert_eq!(fake.calls(), 9);
}

#[test]
fn cancellation_before_and_after_io_discards_audio() {
    let flag = Arc::new(AtomicBool::new(true));
    let fake = Arc::new(FakeTransport::new(vec![FakeTransport::ok(fixture(
        "success.json",
    ))]));
    let synth = adapter(fake.clone());
    let before_io = request("Xin chào");
    let approved = consent(&synth, &before_io, 1_000_000).unwrap();
    assert!(matches!(
        synthesize(&synth, &before_io, &approved, &Hooks(flag)),
        Err(Error::Cancelled)
    ));
    assert_eq!(fake.calls(), 0);

    let flag = Arc::new(AtomicBool::new(false));
    let transport = Arc::new(CancelTransport {
        flag: flag.clone(),
        reply: Mutex::new(Some(FakeTransport::ok(fixture("success.json")))),
    });
    let synth = adapter(transport);
    let after_io = request("Xin chào");
    let approved = consent(&synth, &after_io, 1_000_000).unwrap();
    assert!(matches!(
        synthesize(&synth, &after_io, &approved, &Hooks(flag)),
        Err(Error::Cancelled)
    ));
}

#[test]
fn capabilities_disclose_observed_languages_builtin_voices_and_policy() {
    let fake = Arc::new(FakeTransport::new(Vec::new()));
    let synth = adapter(fake);
    let request = request("Xin chào");
    let caps = synth.capabilities(&request);

    assert_eq!(caps.languages.len(), SPEECH_LANGUAGES.len());
    assert_eq!(caps.voices.len(), SPEECH_VOICES.len());
    assert!(
        caps.voices
            .iter()
            .all(|voice| voice.category == VoiceCategory::BuiltIn)
    );
    assert_eq!(
        caps.outputs,
        vec![SpeechOutput {
            format: SpeechFormat::WavPcm16,
            sample_rate: SPEECH_SAMPLE_RATE
        }]
    );
    assert_eq!(caps.max_characters, None);
    assert_eq!(caps.streaming, ai::Support::Unsupported);
    assert_eq!(
        caps.policy.policy_url.as_str(),
        "https://developers.openai.com/api/docs/guides/your-data"
    );
    let retention = caps.policy.retention.as_str();
    for phrase in [
        "not used to train",
        "30 days",
        "1 hour",
        "Zero Data Retention",
        "not zero",
    ] {
        assert!(
            retention.contains(phrase),
            "missing policy phrase: {phrase}"
        );
    }
    assert!(!retention.to_ascii_lowercase().contains("zero retention"));
    assert!(
        caps.policy
            .voice_disclosure
            .as_str()
            .contains("AI-generated")
    );

    let fingerprint = caps.fingerprint.as_str();
    for part in [
        "gpt-audio-1.5",
        "voice=marin",
        "language=vi",
        "script=Latn",
        "instructions=verbatim-v1",
        "normalization=NFC",
        "format=wav",
    ] {
        assert!(
            fingerprint.contains(part),
            "missing fingerprint part: {part}"
        );
    }
    assert_eq!(
        SPEECH_ENDPOINT,
        "https://api.openai.com/v1/chat/completions"
    );
}

#[test]
fn bounded_transport_rejects_oversized_response_bodies() {
    let fake = FakeTransport::new(vec![FakeTransport::ok(vec![b'x'; 128])]);
    let request = HttpRequest {
        method: "POST",
        url: SPEECH_ENDPOINT.into(),
        headers: Vec::new(),
        body: Vec::new(),
    };
    assert!(matches!(
        fake.send_bounded(request, 64),
        Err(Error::Payload)
    ));
}

#[test]
fn debug_output_redacts_keys_headers_and_audio_bodies() {
    let fake = Arc::new(FakeTransport::new(Vec::new()));
    let synth = adapter(fake);
    assert!(!format!("{synth:?}").contains("test-key-not-real"));
    assert_eq!(
        format!("{:?}", ApiKey::new("secret").unwrap()),
        "ApiKey([redacted])"
    );
    let request = HttpRequest {
        method: "POST",
        url: SPEECH_ENDPOINT.into(),
        headers: vec![("Authorization".into(), "Bearer secret".into())],
        body: b"private text".to_vec(),
    };
    let debug = format!("{request:?}");
    assert!(!debug.contains("secret"));
    assert!(!debug.contains("private text"));
    let response = HttpResponse {
        status: 200,
        headers: vec![("x-request-id".into(), "request".into())],
        body: b"private transcript".to_vec(),
    };
    let debug = format!("{response:?}");
    assert!(!debug.contains("private transcript"));
    assert!(!debug.contains("request"));
}

const LIVE_VI: &str = "Xin chào, đây là một câu kiểm tra giọng nói.";
const LIVE_EN: &str = "Hello, this is a speech synthesis test.";
const LIVE_ZH: &str = "你好，这是一次语音合成测试。";

#[test]
#[ignore = "requires VIBECUT_LIVE_AI=1 and OPENAI_API_KEY; three bounded synthetic speech calls"]
fn live_openai_speech_uses_production_transport() {
    if std::env::var("VIBECUT_LIVE_AI").as_deref() != Ok("1") {
        eprintln!("live test skipped: set VIBECUT_LIVE_AI=1 to run");
        return;
    }
    let key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY");
    let config = config();
    let synth = OpenAiSpeechSynthesizer::new(
        config,
        ApiKey::new(key).unwrap(),
        Arc::new(UreqTransport::new()),
    )
    .unwrap();
    for (text, language, script) in [
        (LIVE_VI, "vi", "Latn"),
        (LIVE_EN, "en", "Latn"),
        (LIVE_ZH, "zh", "Hans"),
    ] {
        let request = request_with(text, "marin", language, script, 15_000_000);
        let approved = consent(&synth, &request, 100_000).unwrap();
        let audio = synthesize(&synth, &request, &approved, &NoHooks).expect("live speech request");
        assert_eq!(audio.transcript.as_ref(), Some(request.text()));
        assert_eq!(audio.sample_rate, SPEECH_SAMPLE_RATE);
        assert!(!audio.samples.is_empty());
        assert!(audio.usage.actual_cost.as_ref().unwrap().micros() <= 100_000);
        eprintln!(
            "live speech verified: language={language} samples={} duration_us={} cost_micros={}",
            audio.samples.len(),
            audio.duration_micros,
            audio.usage.actual_cost.as_ref().unwrap().micros()
        );
    }
}
