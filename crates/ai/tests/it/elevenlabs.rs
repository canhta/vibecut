#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
use ai::foundation::transport::ApiKey;
use ai::providers::elevenlabs::{
    ENDPOINT, ElevenLabsTranscriber, MAX_DURATION_MICROS, MAX_UPLOAD_BYTES,
};
use ai::{
    AudioWindow, Consent, CostEstimate, Error, ExecutionHooks, Money, Operation, ProviderConfig,
    Request, Scope, Stage, TimingUnit, Transcriber,
};
use captions::LanguageTag;
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use transport::{HttpRequest, HttpResponse, HttpTransport, UreqTransport};

struct NoHooks;
impl ExecutionHooks for NoHooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn stage(&self, _: Stage) {}
}

enum Reply {
    Response {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    Failure(transport::Error),
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
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, transport::Error> {
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
            None => Err(transport::Error::Network),
        }
    }
}

fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/elevenlabs")
            .join(name),
    )
    .unwrap()
}

fn config() -> ProviderConfig {
    ElevenLabsTranscriber::provider_config("scribe_v2").unwrap()
}

fn other_config() -> ProviderConfig {
    ai::providers::openai::OpenAiTranscriber::provider_config("gpt-transcribe").unwrap()
}

fn scope(provider: ProviderConfig) -> Scope {
    Scope {
        provider,
        operation: Operation::Transcribe,
        payload_fingerprint: "sha256:audio-fixture".into(),
        mode: "file".into(),
        texts: Vec::new(),
        source_language: LanguageTag::parse("vi").unwrap(),
        target_language: None,
        source_samples: ai::SampleRange::new(0, 32_000).unwrap(),
        sample_rate: 16_000,
        max_bytes: 64_044,
    }
}

fn consent(scope: &Scope) -> Consent {
    Consent::grant(scope.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap()
}

fn adapter(fake: &Arc<FakeTransport>, provider: ProviderConfig) -> ElevenLabsTranscriber {
    ElevenLabsTranscriber::new(
        provider,
        ApiKey::new("test-key-not-real").unwrap(),
        fake.clone(),
    )
    .unwrap()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn multipart(body: &[u8], content_type: &str) -> BTreeMap<String, (String, Vec<u8>)> {
    let boundary = content_type
        .split_once("boundary=")
        .expect("multipart boundary")
        .1;
    let marker = format!("--{boundary}");
    let mut fields = BTreeMap::new();
    let mut rest = body;
    while let Some(start) = find(rest, marker.as_bytes()) {
        rest = &rest[start + marker.len()..];
        if rest.starts_with(b"--") {
            break;
        }
        if rest.starts_with(b"\r\n") {
            rest = &rest[2..];
        }
        let header_end = find(rest, b"\r\n\r\n").expect("part headers");
        let headers = std::str::from_utf8(&rest[..header_end]).unwrap();
        let value_start = header_end + 4;
        let next = find(&rest[value_start..], marker.as_bytes()).expect("next part");
        let value = rest[value_start..value_start + next]
            .strip_suffix(b"\r\n")
            .expect("part terminator");
        let name = headers
            .split(';')
            .find_map(|part| {
                part.trim()
                    .strip_prefix("name=\"")
                    .and_then(|name| name.strip_suffix('"'))
            })
            .expect("part name");
        fields.insert(name.to_owned(), (headers.to_owned(), value.to_vec()));
        rest = &rest[value_start + next..];
    }
    fields
}

fn header<'a>(request: &'a HttpRequest, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|(header, _)| header.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn fixture_scope(name: &str, language: &str) -> (Scope, Vec<f32>, Value) {
    let body = fixture(name);
    let value: Value = serde_json::from_slice(&body).unwrap();
    let duration = value["audio_duration_secs"].as_f64().unwrap();
    let samples = (duration * 16_000.0).ceil() as usize;
    let provider = config();
    let mut scope = scope(provider);
    scope.source_language = LanguageTag::parse(language).unwrap();
    scope.source_samples = ai::SampleRange::new(0, samples as u64).unwrap();
    scope.max_bytes = samples as u64 * 2 + 44;
    (scope, vec![0.0f32; samples], value)
}

#[test]
fn recorded_english_words_become_shared_token_spans() {
    let (scope, samples, value) = fixture_scope("transcribe-en.json", "en");
    let fake = Arc::new(FakeTransport::new(vec![
        FakeTransport::ok(fixture("transcribe-en.json")),
        FakeTransport::ok(fixture("transcribe-en.json")),
    ]));
    let provider = scope.provider.clone();
    let transcriber = adapter(&fake, provider);
    let approved = consent(&scope);
    let request = Request {
        scope: &scope,
        consent: Some(&approved),
    };
    let audio = AudioWindow {
        samples: &samples,
        range: scope.source_samples,
        sample_rate: scope.sample_rate,
    };
    let first = ai::transcribe(&transcriber, audio, &request, None, &NoHooks).unwrap();
    assert!(!first.is_text_only());
    assert_eq!(first.segments().len(), 1);
    let segment = &first.segments()[0];
    let expected_text = value["text"].as_str().unwrap();
    assert_eq!(segment.text.as_str(), expected_text);
    assert_eq!(segment.raw_tokens[0].range.unwrap().start(), 11_520);
    let transcript = first.to_transcript().unwrap();
    let spans = transcript.segments[0].spans.as_ref().unwrap();
    assert_eq!(spans.raw(), expected_text);
    assert!(spans.spans().len() > 10);
    assert!(
        spans
            .spans()
            .iter()
            .all(|span| span.input().unit == TimingUnit::Token)
    );
    assert!(
        spans
            .spans()
            .iter()
            .all(|span| span.input().range.is_some())
    );
    assert!(transcript.segments[0].words.is_empty());

    let second = ai::transcribe(&transcriber, audio, &request, None, &NoHooks).unwrap();
    assert_eq!(first.segments()[0].id, second.segments()[0].id);

    let requests = fake.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let outbound = &requests[0];
    let expected_request: Value = serde_json::from_slice(&fixture("request.json")).unwrap();
    assert_eq!(
        outbound.method,
        expected_request["method"].as_str().unwrap()
    );
    assert_eq!(outbound.url, ENDPOINT);
    assert_eq!(header(outbound, "xi-api-key"), Some("test-key-not-real"));
    assert!(!outbound.url.contains("enable_logging"));
    assert!(!format!("{outbound:?}").contains("test-key-not-real"));
    let content_type = header(outbound, "content-type").unwrap();
    let fields = multipart(&outbound.body, content_type);
    assert_eq!(fields["model_id"].1, b"scribe_v2");
    assert_eq!(fields["language_code"].1, b"en");
    assert_eq!(fields["timestamps_granularity"].1, b"word");
    let (headers, file) = &fields["file"];
    assert!(headers.contains("filename=\"audio.wav\""));
    assert_eq!(file.len(), scope.max_bytes as usize);
    assert_eq!(&file[..4], b"RIFF");
}

#[test]
fn vietnamese_and_chinese_word_tokens_keep_raw_nfc_correspondence() {
    for (name, language, marker) in [
        ("transcribe-vi.json", "vi", "trình diễn"),
        ("transcribe-zh.json", "zh", "特朗普"),
    ] {
        let (scope, samples, value) = fixture_scope(name, language);
        let fake = Arc::new(FakeTransport::new(vec![FakeTransport::ok(fixture(name))]));
        let transcriber = adapter(&fake, scope.provider.clone());
        let approved = consent(&scope);
        let audio = AudioWindow {
            samples: &samples,
            range: scope.source_samples,
            sample_rate: scope.sample_rate,
        };
        let recognition = ai::transcribe(
            &transcriber,
            audio,
            &Request {
                scope: &scope,
                consent: Some(&approved),
            },
            None,
            &NoHooks,
        )
        .unwrap();
        let transcript = recognition.to_transcript().unwrap();
        let spans = transcript.segments[0].spans.as_ref().unwrap();
        assert_eq!(spans.raw(), value["text"].as_str().unwrap());
        assert!(spans.raw().contains(marker));
        assert!(spans.spans().len() > 10);
        let timed = spans
            .spans()
            .iter()
            .filter(|span| span.input().range.is_some())
            .count();
        // Zero-width punctuation stays explicitly untimed; every other token keeps
        // the provider's word/character range.
        assert!(timed * 10 >= spans.spans().len() * 9);
    }
}

#[test]
fn timed_chunks_assemble_in_source_order_and_encoded_boundary_is_checked() {
    let fake = Arc::new(FakeTransport::new(vec![
        FakeTransport::ok(fixture("transcribe-en.json")),
        FakeTransport::ok(fixture("transcribe-en.json")),
        FakeTransport::ok(
            serde_json::to_vec(&serde_json::json!({
                "text": "a",
                "words": [{"text": "a", "start": 0.0, "end": 0.0001, "type": "word"}]
            }))
            .unwrap(),
        ),
    ]));
    let provider = config();
    let transcriber = adapter(&fake, provider.clone());
    let samples = vec![0.0f32; 1_440_000];
    let mut first_scope = scope(provider.clone());
    first_scope.source_language = LanguageTag::parse("en").unwrap();
    first_scope.source_samples = ai::SampleRange::new(0, 1_440_000).unwrap();
    first_scope.max_bytes = 1_440_000 * 2 + 44;
    let first_consent = consent(&first_scope);
    let first = ai::transcribe(
        &transcriber,
        AudioWindow {
            samples: &samples,
            range: first_scope.source_samples,
            sample_rate: 16_000,
        },
        &Request {
            scope: &first_scope,
            consent: Some(&first_consent),
        },
        None,
        &NoHooks,
    )
    .unwrap();

    let mut second_scope = scope(provider);
    second_scope.source_language = LanguageTag::parse("en").unwrap();
    second_scope.source_samples = ai::SampleRange::new(1_440_000, 2_880_000).unwrap();
    second_scope.max_bytes = 1_440_000 * 2 + 44;
    let second_consent = consent(&second_scope);
    let second = ai::transcribe(
        &transcriber,
        AudioWindow {
            samples: &samples,
            range: second_scope.source_samples,
            sample_rate: 16_000,
        },
        &Request {
            scope: &second_scope,
            consent: Some(&second_consent),
        },
        None,
        &NoHooks,
    )
    .unwrap();
    let merged = ai::assemble(vec![first, second], &NoHooks).unwrap();
    assert_eq!(merged.segments().len(), 2);
    assert!(
        merged.segments()[0].range.unwrap().start() < merged.segments()[1].range.unwrap().start()
    );

    let mut exact = scope(config());
    exact.source_samples = ai::SampleRange::new(0, 1).unwrap();
    exact.max_bytes = 46;
    let exact_consent = consent(&exact);
    let one = [0.0f32];
    assert!(
        ai::transcribe(
            &transcriber,
            AudioWindow {
                samples: &one,
                range: exact.source_samples,
                sample_rate: 16_000,
            },
            &Request {
                scope: &exact,
                consent: Some(&exact_consent)
            },
            None,
            &NoHooks,
        )
        .is_ok()
    );
    let mut over = exact;
    over.max_bytes = 45;
    let over_consent = consent(&over);
    assert!(matches!(
        ai::transcribe(
            &transcriber,
            AudioWindow {
                samples: &one,
                range: over.source_samples,
                sample_rate: 16_000,
            },
            &Request {
                scope: &over,
                consent: Some(&over_consent)
            },
            None,
            &NoHooks,
        ),
        Err(Error::Payload)
    ));
}

#[test]
fn disclosure_is_elevenlabs_specific_and_consent_binds_provider_and_payload() {
    let fake = Arc::new(FakeTransport::new(vec![FakeTransport::ok(fixture(
        "transcribe-en.json",
    ))]));
    let provider = config();
    let transcriber = adapter(&fake, provider.clone());
    let scope = scope(provider.clone());

    let disclosure = transcriber.disclosure(&scope).unwrap().unwrap();
    assert_eq!(disclosure.operation, Operation::Transcribe);
    assert_eq!(disclosure.languages.as_str(), "vi");
    assert_eq!(disclosure.audio_micros, 2_000_000);
    assert_eq!(disclosure.upload_bytes, 64_044);
    match &disclosure.estimate {
        CostEstimate::UpperBound(cost) => {
            assert_eq!(cost.currency(), "USD");
            assert_eq!(cost.micros(), 123);
        }
        other => panic!("unexpected estimate: {other:?}"),
    }
    assert!(
        disclosure
            .link("policy")
            .unwrap()
            .as_str()
            .starts_with("https://elevenlabs.io/")
    );
    assert!(disclosure.link("pricing").is_some());
    assert_eq!(disclosure.provider.as_str(), "ElevenLabs");

    let mut translate = scope.clone();
    translate.operation = Operation::Translate;
    translate.target_language = Some(LanguageTag::parse("en").unwrap());
    assert!(matches!(
        transcriber.disclosure(&translate),
        Err(Error::Unsupported)
    ));

    let samples = vec![0.0f32; 32_000];
    let audio = AudioWindow {
        samples: &samples,
        range: scope.source_samples,
        sample_rate: scope.sample_rate,
    };
    assert!(matches!(
        ai::transcribe(
            &transcriber,
            audio,
            &Request {
                scope: &scope,
                consent: None
            },
            None,
            &NoHooks
        ),
        Err(Error::ConsentRequired)
    ));
    assert_eq!(fake.calls(), 0);

    let approved = consent(&scope);
    let mut changed_payload = scope.clone();
    changed_payload.payload_fingerprint = "sha256:different".into();
    assert!(matches!(
        ai::transcribe(
            &transcriber,
            audio,
            &Request {
                scope: &changed_payload,
                consent: Some(&approved)
            },
            None,
            &NoHooks
        ),
        Err(Error::ConsentRequired)
    ));
    assert_eq!(fake.calls(), 0);
    let mut changed_provider = scope.clone();
    changed_provider.provider = other_config();
    let estimate = CostEstimate::UpperBound(Money::new("USD", 123).unwrap());
    assert!(matches!(
        ai::authorize(&changed_payload, &estimate, Some(&approved)),
        Err(Error::ConsentRequired)
    ));
    assert!(matches!(
        ai::authorize(&changed_provider, &estimate, Some(&approved)),
        Err(Error::ConsentRequired)
    ));
    assert!(matches!(
        ai::transcribe(
            &transcriber,
            audio,
            &Request {
                scope: &changed_provider,
                consent: Some(&approved)
            },
            None,
            &NoHooks
        ),
        Err(Error::Unsupported)
    ));
    assert_eq!(fake.calls(), 0);
}

#[test]
fn constructor_refuses_foreign_settings_identity_before_transport() {
    let fake = Arc::new(FakeTransport::new(Vec::new()));
    let canonical = config();
    let transcriber = adapter(&fake, canonical.clone());
    assert_eq!(transcriber.config(), &canonical);
    assert_eq!(
        transcriber.config().settings_fingerprint(),
        canonical.settings_fingerprint()
    );

    let openai = other_config();
    assert_ne!(
        openai.settings_fingerprint(),
        canonical.settings_fingerprint()
    );
    let cases = [
        (
            "unrelated nonempty fingerprint",
            "foreign-transport-v1;model=scribe_v2",
        ),
        (
            "the other provider's canonical fingerprint",
            openai.settings_fingerprint().as_str(),
        ),
    ];
    for (name, settings) in cases {
        let forged = ProviderConfig::new(
            canonical.id().clone(),
            canonical.model().clone(),
            settings.into(),
            canonical.location().clone(),
        )
        .unwrap();
        assert!(
            matches!(
                ElevenLabsTranscriber::new(
                    forged,
                    ApiKey::new("test-key-not-real").unwrap(),
                    fake.clone()
                ),
                Err(Error::Unsupported)
            ),
            "{name} was admitted"
        );
    }
    assert_eq!(fake.calls(), 0);
}

#[test]
fn provider_errors_are_typed_without_bodies_and_do_not_retry() {
    let fake = Arc::new(FakeTransport::new(vec![
        Reply::Response {
            status: 401,
            headers: Vec::new(),
            body: fixture("error-auth.json"),
        },
        Reply::Response {
            status: 402,
            headers: Vec::new(),
            body: fixture("error-payment.json"),
        },
        Reply::Response {
            status: 429,
            headers: vec![("Retry-After".into(), "7".into())],
            body: fixture("error-rate.json"),
        },
        Reply::Response {
            status: 413,
            headers: Vec::new(),
            body: fixture("error-payload.json"),
        },
        Reply::Response {
            status: 500,
            headers: Vec::new(),
            body: fixture("error-server.json"),
        },
        FakeTransport::ok(fixture("error-malformed.json")),
        Reply::Failure(transport::Error::Network),
        Reply::Response {
            status: 503,
            headers: Vec::new(),
            body: fixture("error-server.json"),
        },
        Reply::Response {
            status: 302,
            headers: Vec::new(),
            body: Vec::new(),
        },
        FakeTransport::ok(vec![b'x'; transport::MAX_RESPONSE_BYTES + 1]),
        Reply::Response {
            status: 400,
            headers: Vec::new(),
            body: fixture("error-auth.json"),
        },
        Reply::Response {
            status: 400,
            headers: Vec::new(),
            body: fixture("error-payload.json"),
        },
    ]));
    let provider = config();
    let transcriber = adapter(&fake, provider.clone());
    let scope = scope(provider);
    let approved = consent(&scope);
    let request = Request {
        scope: &scope,
        consent: Some(&approved),
    };
    let samples = vec![0.0f32; 32_000];
    let audio = AudioWindow {
        samples: &samples,
        range: scope.source_samples,
        sample_rate: scope.sample_rate,
    };
    let cases: Vec<Error> = (0..12)
        .map(|_| {
            ai::transcribe(&transcriber, audio, &request, None, &NoHooks)
                .expect_err("fixture must fail")
        })
        .collect();
    assert!(matches!(cases[0], Error::Auth));
    assert!(matches!(cases[1], Error::Quota));
    assert!(matches!(
        cases[2],
        Error::RateLimit {
            retry_after_seconds: Some(7)
        }
    ));
    assert!(matches!(cases[3], Error::Payload));
    assert!(matches!(cases[4], Error::Server));
    assert!(matches!(cases[5], Error::Schema));
    assert!(matches!(cases[6], Error::Network));
    assert!(matches!(cases[7], Error::Server));
    assert!(matches!(cases[8], Error::Invalid(_)));
    assert!(matches!(cases[9], Error::Payload));
    assert!(matches!(cases[10], Error::Auth));
    assert!(matches!(cases[11], Error::Payload));
    for error in &cases {
        let display = error.to_string();
        assert!(!display.contains("xi-secret"));
        assert!(!display.contains("req_secret"));
        assert!(!display.contains("Insufficient credits"));
    }
    assert_eq!(fake.calls(), 12);
}

#[test]
fn hostile_timing_and_text_mismatch_are_rejected() {
    let fake = Arc::new(FakeTransport::new(vec![
        FakeTransport::ok(
            serde_json::to_vec(&serde_json::json!({
                "text": "a",
                "words": [{"text": "a", "start": 0.0, "end": 999.0, "type": "word"}]
            }))
            .unwrap(),
        ),
        FakeTransport::ok(
            serde_json::to_vec(&serde_json::json!({
                "text": "a b",
                "words": [
                    {"text": "a", "start": 1.0, "end": 1.2, "type": "word"},
                    {"text": "b", "start": 0.5, "end": 0.7, "type": "word"}
                ]
            }))
            .unwrap(),
        ),
        FakeTransport::ok(
            serde_json::to_vec(&serde_json::json!({
                "text": "different",
                "words": [{"text": "a", "start": 0.1, "end": 0.2, "type": "word"}]
            }))
            .unwrap(),
        ),
    ]));
    let provider = config();
    let transcriber = adapter(&fake, provider.clone());
    let scope = scope(provider);
    let approved = consent(&scope);
    let request = Request {
        scope: &scope,
        consent: Some(&approved),
    };
    let samples = vec![0.0f32; 32_000];
    let audio = AudioWindow {
        samples: &samples,
        range: scope.source_samples,
        sample_rate: scope.sample_rate,
    };
    for _ in 0..3 {
        assert!(matches!(
            ai::transcribe(&transcriber, audio, &request, None, &NoHooks),
            Err(Error::Schema)
        ));
    }
}

#[test]
fn capabilities_admit_timed_tokens_and_document_limits() {
    let fake = Arc::new(FakeTransport::new(Vec::new()));
    let provider = config();
    let transcriber = adapter(&fake, provider.clone());
    let scope = scope(provider);
    let capabilities = transcriber.capabilities(&scope);
    assert!(capabilities.timing.contains(&TimingUnit::Segment));
    assert!(capabilities.timing.contains(&TimingUnit::Token));
    assert_eq!(capabilities.max_upload_bytes, Some(MAX_UPLOAD_BYTES));
    assert_eq!(capabilities.max_duration_micros, Some(MAX_DURATION_MICROS));
    assert_eq!(capabilities.languages.len(), 3);

    let mut wrong_mode = scope.clone();
    wrong_mode.mode = "live".into();
    assert!(matches!(
        capabilities.check(&wrong_mode),
        Err(Error::Unsupported)
    ));
    let mut wrong_language = scope.clone();
    wrong_language.source_language = LanguageTag::parse("fr").unwrap();
    assert!(matches!(
        capabilities.check(&wrong_language),
        Err(Error::UnsupportedLanguage)
    ));
    let mut too_large = scope;
    too_large.max_bytes = MAX_UPLOAD_BYTES + 1;
    assert!(matches!(
        capabilities.check(&too_large),
        Err(Error::Payload)
    ));
}

#[test]
fn debug_output_redacts_key_headers_audio_and_transcript_bodies() {
    let fake = Arc::new(FakeTransport::new(Vec::new()));
    let provider = config();
    let transcriber = adapter(&fake, provider);
    assert!(!format!("{transcriber:?}").contains("test-key-not-real"));
    assert_eq!(
        format!("{:?}", ApiKey::new("secret").unwrap()),
        "ApiKey([redacted])"
    );
    let request = HttpRequest {
        method: "POST",
        url: ENDPOINT.into(),
        headers: vec![("xi-api-key".into(), "secret".into())],
        body: b"audio bytes".to_vec(),
    };
    let debug = format!("{request:?}");
    assert!(!debug.contains("secret"));
    assert!(!debug.contains("audio bytes"));
    let response = HttpResponse {
        status: 200,
        headers: vec![("request-id".into(), "secret".into())],
        body: b"private transcript".to_vec(),
    };
    let debug = format!("{response:?}");
    assert!(!debug.contains("private transcript"));
    assert!(!debug.contains("secret"));
}

const LIVE_SPEECH: &str = "The quick brown fox jumps over the lazy dog. Today we are testing speech recognition in a video editor.";

fn live_speech() -> Option<Vec<f32>> {
    fn decode(bytes: &[u8]) -> Vec<f32> {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| f32::from_le_bytes(*chunk))
            .collect()
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let recorded = root.join("../../target/fixtures/openai-review/speech.f32");
    if let Ok(bytes) = fs::read(&recorded) {
        return Some(decode(&bytes));
    }
    let dir = root.join("../../target/fixtures/elevenlabs-live");
    fs::create_dir_all(&dir).ok()?;
    let aiff = dir.join("speech.aiff");
    let pcm = dir.join("speech.f32");
    let status = std::process::Command::new("say")
        .arg("-v")
        .arg("Samantha")
        .arg("-o")
        .arg(&aiff)
        .arg(LIVE_SPEECH)
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    let status = testkit::oracle::command("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&aiff)
        .args(["-ar", "16000", "-ac", "1", "-f", "f32le"])
        .arg(&pcm)
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    fs::read(&pcm).ok().map(|bytes| decode(&bytes))
}

#[test]
#[ignore = "requires VIBECUT_LIVE_AI=1 and ELEVENLABS_API_KEY; uploads six seconds of synthetic speech"]
fn live_elevenlabs_transcription_uses_production_transport() {
    if std::env::var("VIBECUT_LIVE_AI").as_deref() != Ok("1") {
        eprintln!("live test skipped: set VIBECUT_LIVE_AI=1 to run");
        return;
    }
    let key = std::env::var("ELEVENLABS_API_KEY").expect("ELEVENLABS_API_KEY");
    let samples = live_speech().expect("live speech probe unavailable; see fixtures README");
    assert!(samples.len() > 16_000);
    assert!(samples.iter().any(|value| value.abs() > 0.1));
    let provider = config();
    let transcriber = ElevenLabsTranscriber::new(
        provider.clone(),
        ApiKey::new(key).unwrap(),
        Arc::new(UreqTransport::new()),
    )
    .unwrap();
    let mut scope = scope(provider);
    scope.source_language = LanguageTag::parse("en").unwrap();
    scope.source_samples = ai::SampleRange::new(0, samples.len() as u64).unwrap();
    scope.max_bytes = samples.len() as u64 * 2 + 44;
    let approved = Consent::grant(scope.clone(), Money::new("USD", 10_000).unwrap()).unwrap();
    let request = Request {
        scope: &scope,
        consent: Some(&approved),
    };
    let audio = AudioWindow {
        samples: &samples,
        range: scope.source_samples,
        sample_rate: scope.sample_rate,
    };
    let recognition =
        ai::transcribe(&transcriber, audio, &request, None, &NoHooks).expect("live speech request");
    assert!(!recognition.is_text_only());
    assert!(!recognition.segments().is_empty());
    let transcript = recognition.to_transcript().expect("timed transcript");
    let spans = transcript.segments[0].spans.as_ref().expect("word spans");
    assert!(!spans.spans().is_empty());
    let text = recognition
        .segments()
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    for anchor in [
        "quick",
        "brown",
        "fox",
        "lazy",
        "dog",
        "speech",
        "recognition",
        "video",
        "editor",
    ] {
        assert!(text.contains(anchor), "spoken anchor missing: {anchor}");
    }
    eprintln!(
        "live speech verified: {} samples, {} tokens, 9 anchors; no transcript logged",
        samples.len(),
        spans.spans().len()
    );
}

#[test]
fn audio_events_keep_their_own_native_timing() {
    for words in [
        serde_json::json!([
            {"text":"Hello", "start":0.1, "end":0.5, "type":"word"},
            {"text":" ", "start":0.5, "end":1.0, "type":"spacing"},
            {"text":"(laughter)", "start":1.0, "end":1.8, "type":"audio_event"}
        ]),
        serde_json::json!([
            {"text":"(laughter)", "start":0.1, "end":0.5, "type":"audio_event"},
            {"text":" ", "start":0.5, "end":1.0, "type":"spacing"},
            {"text":"(applause)", "start":1.0, "end":1.8, "type":"audio_event"}
        ]),
    ] {
        let text = words
            .as_array()
            .unwrap()
            .iter()
            .map(|word| word["text"].as_str().unwrap())
            .collect::<String>();
        let fake = Arc::new(FakeTransport::new(vec![FakeTransport::ok(
            serde_json::to_vec(&serde_json::json!({"text":text, "words":words})).unwrap(),
        )]));
        let scope = scope(config());
        let transcriber = adapter(&fake, scope.provider.clone());
        let approved = consent(&scope);
        let samples = vec![0.0; 32_000];
        let recognition = ai::transcribe(
            &transcriber,
            AudioWindow {
                samples: &samples,
                range: scope.source_samples,
                sample_rate: scope.sample_rate,
            },
            &Request {
                scope: &scope,
                consent: Some(&approved),
            },
            None,
            &NoHooks,
        )
        .unwrap();
        let segment = &recognition.segments()[0];
        assert_eq!(
            segment.range.unwrap().end(),
            28_800,
            "trailing audio event must extend the segment"
        );
        assert_eq!(segment.raw_tokens.len(), 2);
        assert_eq!(
            segment.raw_tokens[1].range,
            Some(ai::SampleRange::new(16_000, 28_800).unwrap())
        );
        assert_eq!(
            segment.raw_tokens[1].bytes,
            words[2]["text"].as_str().unwrap().as_bytes()
        );
        assert!(recognition.to_transcript().is_ok());
    }
}
