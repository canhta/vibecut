#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
use ai::openai::{MAX_UPLOAD_BYTES, MODEL, OpenAiTranscriber};
use ai::transport::{ApiKey, HttpRequest, HttpResponse, HttpTransport, UreqTransport};
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

fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/openai")
            .join(name),
    )
    .unwrap()
}

fn config() -> ProviderConfig {
    OpenAiTranscriber::provider_config(MODEL).unwrap()
}

fn other_config() -> ProviderConfig {
    ai::elevenlabs::ElevenLabsTranscriber::provider_config(ai::elevenlabs::MODEL).unwrap()
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
        source_samples: ai::SampleRange::new(16_000, 48_000).unwrap(),
        sample_rate: 16_000,
        max_bytes: 128_000,
    }
}

fn consent(scope: &Scope) -> Consent {
    Consent::grant(scope.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap()
}

fn adapter(fake: &Arc<FakeTransport>, provider: ProviderConfig) -> OpenAiTranscriber {
    OpenAiTranscriber::new(
        provider,
        ApiKey::new("test-key-not-real").unwrap(),
        fake.clone(),
    )
    .unwrap()
}

fn pcm() -> Vec<f32> {
    vec![0.25; 32_000]
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

#[test]
fn consented_upload_uses_recorded_fixture_and_stable_text_only_ids() {
    let fake = Arc::new(FakeTransport::new(vec![
        FakeTransport::ok(fixture("transcribe-success.json")),
        FakeTransport::ok(fixture("transcribe-success.json")),
    ]));
    let provider = config();
    let transcriber = adapter(&fake, provider.clone());
    let scope = scope(provider);
    let approved = consent(&scope);
    let request = Request {
        scope: &scope,
        consent: Some(&approved),
    };
    let samples = pcm();
    let audio = AudioWindow {
        samples: &samples,
        range: scope.source_samples,
        sample_rate: scope.sample_rate,
    };

    let first = ai::transcribe(&transcriber, audio, &request, None, &NoHooks).unwrap();
    assert!(first.is_text_only());
    assert_eq!(first.segments().len(), 1);
    assert!(matches!(
        first.to_transcript(),
        Err(Error::TimingUnavailable)
    ));
    let expected: Value = serde_json::from_slice(&fixture("transcribe-success.json")).unwrap();
    let expected_text = expected["text"].as_str().unwrap();
    assert_eq!(first.segments()[0].text.as_str(), expected_text);
    assert!(
        first.segments()[0]
            .text
            .as_str()
            .starts_with("Với trình diễn")
    );

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
    assert_eq!(outbound.url, expected_request["url"].as_str().unwrap());
    assert_eq!(
        header(outbound, "authorization"),
        Some("Bearer test-key-not-real")
    );
    let content_type = header(outbound, "content-type").unwrap();
    assert!(content_type.starts_with("multipart/form-data; boundary="));
    assert!(!format!("{outbound:?}").contains("test-key-not-real"));
    let fields = multipart(&outbound.body, content_type);
    for pair in expected_request["fields"].as_array().unwrap() {
        let name = pair[0].as_str().unwrap();
        let value = pair[1].as_str().unwrap();
        assert_eq!(fields[name].1, value.as_bytes(), "field {name}");
    }
    let (headers, file) = &fields["file"];
    assert!(headers.contains("filename=\"audio.wav\""));
    assert!(headers.contains("Content-Type: audio/wav"));
    assert_eq!(file.len(), 64_044);
    assert_eq!(&file[..4], b"RIFF");
    assert_eq!(&file[8..12], b"WAVE");
    assert_eq!(u16::from_le_bytes([file[20], file[21]]), 1);
    assert_eq!(u16::from_le_bytes([file[22], file[23]]), 1);
    assert_eq!(
        u32::from_le_bytes([file[24], file[25], file[26], file[27]]),
        16_000
    );
    assert_eq!(&file[36..40], b"data");
    assert_eq!(
        u32::from_le_bytes([file[40], file[41], file[42], file[43]]),
        64_000
    );
    assert_eq!(&file[44..46], &[0, 32]);
}

#[test]
fn adapter_chunks_assemble_in_source_order_without_invented_timing() {
    let fake = Arc::new(FakeTransport::new(vec![
        FakeTransport::ok(fixture("transcribe-success.json")),
        FakeTransport::ok(fixture("transcribe-empty.json")),
    ]));
    let provider = config();
    let transcriber = adapter(&fake, provider.clone());
    let samples = pcm();
    let first_scope = scope(provider.clone());
    let first_consent = consent(&first_scope);
    let first_request = Request {
        scope: &first_scope,
        consent: Some(&first_consent),
    };
    let first = ai::transcribe(
        &transcriber,
        AudioWindow {
            samples: &samples,
            range: first_scope.source_samples,
            sample_rate: first_scope.sample_rate,
        },
        &first_request,
        None,
        &NoHooks,
    )
    .unwrap();
    let first_id = first.segments()[0].id.clone();

    let mut second_scope = scope(provider);
    second_scope.source_samples = ai::SampleRange::new(48_000, 80_000).unwrap();
    let second_consent = consent(&second_scope);
    let second_request = Request {
        scope: &second_scope,
        consent: Some(&second_consent),
    };
    let second = ai::transcribe(
        &transcriber,
        AudioWindow {
            samples: &samples,
            range: second_scope.source_samples,
            sample_rate: second_scope.sample_rate,
        },
        &second_request,
        None,
        &NoHooks,
    )
    .unwrap();

    let merged = ai::assemble(vec![first, second], &NoHooks).unwrap();
    assert!(merged.is_text_only());
    assert_eq!(
        merged.window(),
        ai::SampleRange::new(16_000, 80_000).unwrap()
    );
    assert_eq!(merged.segments().len(), 1);
    assert_eq!(merged.segments()[0].id, first_id);
}

#[test]
fn no_network_before_consent_and_changed_payload_or_provider_is_refused() {
    let fake = Arc::new(FakeTransport::new(vec![FakeTransport::ok(fixture(
        "transcribe-success.json",
    ))]));
    let provider = config();
    let transcriber = adapter(&fake, provider.clone());
    let scope = scope(provider);
    let samples = pcm();
    let audio = AudioWindow {
        samples: &samples,
        range: scope.source_samples,
        sample_rate: scope.sample_rate,
    };

    let unapproved = Request {
        scope: &scope,
        consent: None,
    };
    assert!(matches!(
        ai::transcribe(&transcriber, audio, &unapproved, None, &NoHooks),
        Err(Error::ConsentRequired)
    ));
    assert_eq!(fake.calls(), 0);

    let approved = consent(&scope);
    let mut changed_payload = scope.clone();
    changed_payload.payload_fingerprint = "sha256:different-audio".into();
    let changed_payload_request = Request {
        scope: &changed_payload,
        consent: Some(&approved),
    };
    assert!(matches!(
        ai::transcribe(
            &transcriber,
            audio,
            &changed_payload_request,
            None,
            &NoHooks
        ),
        Err(Error::ConsentRequired)
    ));
    assert_eq!(fake.calls(), 0);

    let mut changed_provider = scope.clone();
    changed_provider.provider = other_config();
    let changed_provider_request = Request {
        scope: &changed_provider,
        consent: Some(&approved),
    };
    assert!(matches!(
        ai::transcribe(
            &transcriber,
            audio,
            &changed_provider_request,
            None,
            &NoHooks
        ),
        Err(Error::Unsupported)
    ));
    assert_eq!(fake.calls(), 0);
    let estimate = CostEstimate::UpperBound(Money::new("USD", 150).unwrap());
    assert!(matches!(
        ai::authorize(&changed_payload, &estimate, Some(&approved)),
        Err(Error::ConsentRequired)
    ));
    assert!(matches!(
        ai::authorize(&changed_provider, &estimate, Some(&approved)),
        Err(Error::ConsentRequired)
    ));

    let request = Request {
        scope: &scope,
        consent: Some(&approved),
    };
    assert!(ai::transcribe(&transcriber, audio, &request, None, &NoHooks).is_ok());
    assert_eq!(fake.calls(), 1);

    // A WAV container cannot fit a one-sample byte ceiling, so it refuses before
    // sending rather than exceeding the approved upload size.
    let mut tiny_scope = scope.clone();
    tiny_scope.source_samples = ai::SampleRange::new(16_000, 16_001).unwrap();
    tiny_scope.max_bytes = 4;
    let tiny_consent = consent(&tiny_scope);
    let tiny_request = Request {
        scope: &tiny_scope,
        consent: Some(&tiny_consent),
    };
    let tiny_samples = [0.0f32];
    assert!(matches!(
        ai::transcribe(
            &transcriber,
            AudioWindow {
                samples: &tiny_samples,
                range: tiny_scope.source_samples,
                sample_rate: tiny_scope.sample_rate,
            },
            &tiny_request,
            None,
            &NoHooks,
        ),
        Err(Error::Payload)
    ));
    assert_eq!(fake.calls(), 1);
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

    let elevenlabs = other_config();
    assert_ne!(
        elevenlabs.settings_fingerprint(),
        canonical.settings_fingerprint()
    );
    let cases = [
        (
            "unrelated nonempty fingerprint",
            "foreign-transport-v1;model=gpt-transcribe",
        ),
        (
            "the other provider's canonical fingerprint",
            elevenlabs.settings_fingerprint().as_str(),
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
                OpenAiTranscriber::new(
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
fn disclosure_states_duration_size_cost_and_transcription_specific_terms() {
    let fake = Arc::new(FakeTransport::new(Vec::new()));
    let provider = config();
    let transcriber = adapter(&fake, provider.clone());
    let scope = scope(provider);

    let disclosure = transcriber.disclosure(&scope).unwrap().unwrap();
    assert_eq!(disclosure.operation, Operation::Transcribe);
    assert_eq!(disclosure.audio_micros, 2_000_000);
    assert_eq!(disclosure.upload_bytes, 64_044);
    assert_eq!(disclosure.languages.as_str(), "vi");
    match &disclosure.estimate {
        CostEstimate::UpperBound(cost) => {
            assert_eq!(cost.currency(), "USD");
            assert_eq!(cost.micros(), 150);
        }
        other => panic!("unexpected estimate: {other:?}"),
    }
    assert_eq!(
        disclosure.link("policy").unwrap().as_str(),
        "https://developers.openai.com/api/docs/guides/your-data"
    );
    assert_eq!(disclosure.provider.as_str(), "OpenAI");
    assert_eq!(
        disclosure.link("docs").unwrap().as_str(),
        "https://developers.openai.com/api/docs/guides/speech-to-text"
    );

    let mut translate = scope.clone();
    translate.operation = Operation::Translate;
    translate.target_language = Some(LanguageTag::parse("en").unwrap());
    assert!(matches!(
        transcriber.disclosure(&translate),
        Err(Error::Unsupported)
    ));

    let mut changed_provider = scope;
    changed_provider.provider = other_config();
    assert!(matches!(
        transcriber.disclosure(&changed_provider),
        Err(Error::Unsupported)
    ));
}

#[test]
fn advertised_wav_limit_is_usable_despite_larger_f32_memory() {
    let fake = Arc::new(FakeTransport::new(vec![FakeTransport::ok(fixture(
        "transcribe-empty.json",
    ))]));
    let provider = config();
    let transcriber = adapter(&fake, provider.clone());
    let mut scope = scope(provider);
    let samples = vec![0.0f32; 6_400_000];
    scope.source_samples = ai::SampleRange::new(0, samples.len() as u64).unwrap();
    scope.max_bytes = MAX_UPLOAD_BYTES;
    let disclosure = transcriber.disclosure(&scope).unwrap().unwrap();
    assert_eq!(disclosure.upload_bytes, 12_800_044);
    let approved = consent(&scope);
    let request = Request {
        scope: &scope,
        consent: Some(&approved),
    };
    let result = ai::transcribe(
        &transcriber,
        AudioWindow {
            samples: &samples,
            range: scope.source_samples,
            sample_rate: scope.sample_rate,
        },
        &request,
        None,
        &NoHooks,
    );
    assert!(
        result.is_ok(),
        "file below the disclosed 25 MB limit was rejected"
    );
    assert_eq!(fake.calls(), 1);
}

#[test]
fn encoded_wav_ceiling_includes_header_overhead_and_is_exact() {
    let fake = Arc::new(FakeTransport::new(vec![
        FakeTransport::ok(fixture("transcribe-empty.json")),
        FakeTransport::ok(fixture("transcribe-empty.json")),
    ]));
    let provider = config();
    let transcriber = adapter(&fake, provider.clone());

    let mut exact = scope(provider.clone());
    let samples = vec![0.0f32; 4_978];
    exact.source_samples = ai::SampleRange::new(0, samples.len() as u64).unwrap();
    exact.max_bytes = 10_000;
    let approved = consent(&exact);
    assert!(
        ai::transcribe(
            &transcriber,
            AudioWindow {
                samples: &samples,
                range: exact.source_samples,
                sample_rate: exact.sample_rate,
            },
            &Request {
                scope: &exact,
                consent: Some(&approved)
            },
            None,
            &NoHooks,
        )
        .is_ok()
    );

    let mut over = scope(provider);
    let samples = vec![0.0f32; 4_978];
    over.source_samples = ai::SampleRange::new(0, samples.len() as u64).unwrap();
    over.max_bytes = 9_999;
    let approved = consent(&over);
    assert!(matches!(
        ai::transcribe(
            &transcriber,
            AudioWindow {
                samples: &samples,
                range: over.source_samples,
                sample_rate: over.sample_rate,
            },
            &Request {
                scope: &over,
                consent: Some(&approved)
            },
            None,
            &NoHooks,
        ),
        Err(Error::Payload)
    ));
    assert_eq!(fake.calls(), 1);
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
        Reply::Failure(Error::Network),
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
        FakeTransport::ok(vec![b'x'; ai::transport::MAX_RESPONSE_BYTES + 1]),
    ]));
    let provider = config();
    let transcriber = adapter(&fake, provider.clone());
    let scope = scope(provider);
    let approved = consent(&scope);
    let request = Request {
        scope: &scope,
        consent: Some(&approved),
    };
    let samples = pcm();
    let audio = AudioWindow {
        samples: &samples,
        range: scope.source_samples,
        sample_rate: scope.sample_rate,
    };

    let cases: Vec<Error> = (0..10)
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
    assert!(!cases[0].is_transient());
    assert!(!cases[1].is_transient());
    assert!(cases[2].is_transient());
    assert!(!cases[3].is_transient());
    assert!(cases[4].is_transient());
    assert!(cases[6].is_transient());
    assert!(cases[7].is_transient());
    for error in &cases {
        let display = error.to_string();
        assert!(!display.contains("sk-secret"));
        assert!(!display.contains("exceeded your current quota"));
        assert!(!display.contains("Rate limit reached"));
    }
    assert_eq!(fake.calls(), 10);
}

#[test]
fn empty_speech_returns_no_segments_and_capabilities_refuse_wrong_scope() {
    let fake = Arc::new(FakeTransport::new(vec![
        FakeTransport::ok(fixture("transcribe-empty.json")),
        FakeTransport::ok(fixture("transcribe-success.json")),
    ]));
    let provider = config();
    let transcriber = adapter(&fake, provider.clone());
    let scope = scope(provider);
    let approved = consent(&scope);
    let request = Request {
        scope: &scope,
        consent: Some(&approved),
    };
    let samples = pcm();
    let audio = AudioWindow {
        samples: &samples,
        range: scope.source_samples,
        sample_rate: scope.sample_rate,
    };
    let empty = ai::transcribe(&transcriber, audio, &request, None, &NoHooks).unwrap();
    assert!(empty.segments().is_empty());
    assert!(!empty.is_text_only());

    let capabilities = transcriber.capabilities(&scope);
    assert!(capabilities.timing.is_empty());
    assert!(!capabilities.timing.contains(&TimingUnit::Word));
    assert_eq!(capabilities.max_upload_bytes, Some(MAX_UPLOAD_BYTES));
    assert_eq!(capabilities.languages.len(), 3);
    capabilities.check(&scope).unwrap();

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

    let mut too_large = scope.clone();
    too_large.max_bytes = MAX_UPLOAD_BYTES + 1;
    assert!(matches!(
        capabilities.check(&too_large),
        Err(Error::Payload)
    ));
}

#[test]
fn debug_output_redacts_keys_headers_audio_and_transcript_bodies() {
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
        url: "https://api.openai.com/v1/audio/transcriptions".into(),
        headers: vec![("Authorization".into(), "Bearer secret".into())],
        body: b"audio bytes".to_vec(),
    };
    let debug = format!("{request:?}");
    assert!(!debug.contains("secret"));
    assert!(!debug.contains("audio bytes"));
    let response = HttpResponse {
        status: 200,
        headers: vec![("x-request-id".into(), "request".into())],
        body: b"private transcript".to_vec(),
    };
    let debug = format!("{response:?}");
    assert!(!debug.contains("private transcript"));
    assert!(!debug.contains("request"));
}

const LIVE_SPEECH: &str = "The quick brown fox jumps over the lazy dog. Today we are testing speech recognition in a video editor.";

/// Load the bounded synthetic speech probe, or generate it with the documented
/// macOS `say` + ffmpeg command when the reviewer fixture is unavailable.
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
    let dir = root.join("../../target/fixtures/openai-live");
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
#[ignore = "requires VIBECUT_LIVE_AI=1 and OPENAI_API_KEY; uploads six seconds of synthetic speech"]
fn live_openai_transcription_uses_production_transport() {
    if std::env::var("VIBECUT_LIVE_AI").as_deref() != Ok("1") {
        eprintln!("live test skipped: set VIBECUT_LIVE_AI=1 to run");
        return;
    }
    let key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY");
    let samples = live_speech().expect("live speech probe unavailable; see fixtures README");
    assert!(samples.len() > 16_000, "speech probe is too short");
    assert!(
        samples.iter().any(|value| value.abs() > 0.1),
        "speech probe is silent"
    );
    let provider = config();
    let transcriber = OpenAiTranscriber::new(
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
    assert!(recognition.is_text_only());
    assert!(
        !recognition.segments().is_empty(),
        "speech recognized as empty"
    );
    assert!(
        recognition
            .segments()
            .iter()
            .all(|segment| segment.range.is_none())
    );
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
        "live speech verified: {} samples, {} segments, 9 anchors; no transcript logged",
        samples.len(),
        recognition.segments().len()
    );
}
