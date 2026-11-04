#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
use ai::elevenlabs_speech::{
    BASE_URL, ElevenLabsSpeech, ElevenLabsSpeechSettings, FLASH_MICROS_PER_CHARACTER, FLASH_MODEL,
    LoggingMode, MULTILINGUAL_MICROS_PER_CHARACTER, MULTILINGUAL_MODEL, SAMPLE_RATE, SpeechSpeed,
};
use ai::speech::{
    SpeechFormat, SpeechLanguage, SpeechOutput, SpeechRequest, SpeechSynthesizer, SpeechVoice,
    SynthesisConsent, SynthesisEstimate, VoiceCategory, synthesize,
};
use ai::transport::{ApiKey, HttpRequest, HttpResponse, HttpTransport, UreqTransport};
use ai::{Error, ExecutionHooks, Location, Money, ProviderConfig, Stage};
use captions::{LanguageTag, NfcText};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const VOICE: &str = "21m00Tcm4TlvDq8ikWAM";

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
    cancel: Option<(Arc<AtomicBool>, usize)>,
}
impl FakeTransport {
    fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            requests: Mutex::new(Vec::new()),
            cancel: None,
        }
    }
    fn cancelling(replies: Vec<Reply>, flag: Arc<AtomicBool>, after: usize) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            requests: Mutex::new(Vec::new()),
            cancel: Some((flag, after)),
        }
    }
    fn calls(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
    fn take(&self) -> Vec<HttpRequest> {
        std::mem::take(&mut *self.requests.lock().unwrap())
    }
}
impl HttpTransport for FakeTransport {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
        let calls = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            requests.len()
        };
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Reply::Failure(Error::Network));
        if let Some((flag, after)) = &self.cancel
            && calls >= *after
        {
            flag.store(true, Ordering::SeqCst);
        }
        match reply {
            Reply::Response {
                status,
                headers,
                body,
            } => Ok(HttpResponse {
                status,
                headers,
                body,
            }),
            Reply::Failure(error) => Err(error),
        }
    }
}

struct Hooks {
    cancelled: Arc<AtomicBool>,
}
impl Hooks {
    fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
    fn shared(flag: Arc<AtomicBool>) -> Self {
        Self { cancelled: flag }
    }
}
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
    fn stage(&self, _: Stage) {}
}

fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/elevenlabs-speech")
            .join(name),
    )
    .unwrap()
}

fn fixture_json(name: &str) -> Value {
    serde_json::from_slice(&fixture(name)).unwrap()
}

/// The WAV container holds the recorded PCM; the API's `pcm_24000` response is the
/// headerless `data` chunk, so tests replay that slice as the body.
fn pcm(name: &str) -> Vec<u8> {
    let bytes = fixture(name);
    let start = bytes
        .windows(4)
        .position(|window| window == b"data")
        .unwrap()
        + 8;
    let length = u32::from_le_bytes([
        bytes[start - 4],
        bytes[start - 3],
        bytes[start - 2],
        bytes[start - 1],
    ]) as usize;
    bytes[start..start + length].to_vec()
}

fn money(micros: u64) -> Money {
    Money::new("USD", micros).unwrap()
}

fn settings() -> ElevenLabsSpeechSettings {
    ElevenLabsSpeechSettings::default()
}

fn provider_config(
    model: &str,
    settings: ElevenLabsSpeechSettings,
    base_url: &str,
) -> ProviderConfig {
    ProviderConfig::new(
        "elevenlabs-speech".into(),
        model.into(),
        settings.settings_fingerprint().into(),
        Location::Remote {
            backend: "elevenlabs".into(),
            base_url: base_url.into(),
            auth_ref: "keychain:vibecut/elevenlabs".into(),
        },
    )
    .unwrap()
}

fn voices() -> Vec<SpeechVoice> {
    vec![SpeechVoice {
        id: NfcText::from(VOICE),
        category: VoiceCategory::Stock,
    }]
}

fn build_adapter(
    fake: Arc<FakeTransport>,
    model: &str,
    settings: ElevenLabsSpeechSettings,
    stock: Vec<SpeechVoice>,
) -> ElevenLabsSpeech {
    ElevenLabsSpeech::new(
        provider_config(model, settings, BASE_URL),
        ApiKey::new("test-key-not-real").unwrap(),
        fake,
        settings,
        stock,
    )
    .unwrap()
}

fn language(tag: &str, script: &str) -> SpeechLanguage {
    SpeechLanguage {
        language: LanguageTag::parse(tag).unwrap(),
        script: NfcText::from(script),
    }
}

fn build_request(
    provider: ProviderConfig,
    text: &str,
    voice: &str,
    language: SpeechLanguage,
    output: SpeechOutput,
    max_duration_micros: u64,
) -> SpeechRequest {
    SpeechRequest::new(
        provider,
        text.into(),
        voice.into(),
        language,
        output,
        max_duration_micros,
    )
    .unwrap()
}

fn output() -> SpeechOutput {
    SpeechOutput {
        format: SpeechFormat::Pcm16,
        sample_rate: SAMPLE_RATE,
    }
}

fn pcm_reply(pcm: Vec<u8>, character_cost: Option<u64>) -> Reply {
    let mut headers = vec![
        ("content-type".into(), "audio/pcm".into()),
        ("content-length".into(), pcm.len().to_string()),
    ];
    if let Some(cost) = character_cost {
        headers.push(("character-cost".into(), cost.to_string()));
    }
    Reply::Response {
        status: 200,
        headers,
        body: pcm,
    }
}

fn error_reply(file: &str) -> Reply {
    let value = fixture_json(file);
    let status = value["status"].as_u64().unwrap() as u16;
    let mut headers = Vec::new();
    if let Some(retry) = value["retry_after"].as_str() {
        headers.push(("retry-after".into(), retry.into()));
    }
    let body = serde_json::to_vec(&json!({ "detail": value["detail"].clone() })).unwrap();
    Reply::Response {
        status,
        headers,
        body,
    }
}

fn grant_consent(
    adapter: &ElevenLabsSpeech,
    request: &SpeechRequest,
    budget: u64,
) -> SynthesisConsent {
    SynthesisConsent::grant(
        request,
        &adapter.capabilities(request),
        adapter.estimate(request),
        money(budget),
    )
    .unwrap()
}

fn body(value: &HttpRequest) -> Value {
    serde_json::from_slice(&value.body).unwrap()
}

#[test]
fn success_uses_production_seam_and_exact_request() {
    let fixture = fixture_json("success.json");
    let text = fixture["text"].as_str().unwrap();
    let character_cost = fixture["character_cost"].as_u64().unwrap();
    let fake = Arc::new(FakeTransport::new(vec![pcm_reply(
        pcm("success.wav"),
        Some(character_cost),
    )]));
    let adapter = build_adapter(fake.clone(), FLASH_MODEL, settings(), voices());
    let request = build_request(
        adapter.config().clone(),
        text,
        VOICE,
        language("vi", "Latn"),
        output(),
        1_000_000,
    );
    let approved = grant_consent(&adapter, &request, 1_000_000);
    let audio = synthesize(&adapter, &request, &approved, &Hooks::new()).unwrap();

    assert_eq!(audio.samples.len(), 6000);
    assert_eq!(audio.sample_rate, SAMPLE_RATE);
    assert_eq!(audio.duration_micros, 250_000);
    assert_eq!(audio.transcript, None);
    assert!(audio.samples.iter().all(|sample| sample.is_finite()));
    assert_eq!(audio.usage.billable_characters, Some(13));
    assert_eq!(audio.usage.actual_cost, money(520));
    assert_eq!(
        audio.fingerprint,
        adapter.capabilities(&request).fingerprint
    );
    assert_eq!(
        adapter.estimate(&request),
        SynthesisEstimate::UpperBound(money(640))
    );
    // The contract counts extended graphemes; provider billing counts scalars.
    assert_eq!(request.characters(), 10);

    let calls = fake.take();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.method, "POST");
    assert_eq!(
        call.url,
        format!("{BASE_URL}/text-to-speech/{VOICE}?output_format=pcm_24000")
    );
    assert!(call.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("xi-api-key") && value == "test-key-not-real"
    }));
    assert!(call.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("content-type") && value == "application/json"
    }));
    let value = body(call);
    assert_eq!(value["text"], text);
    assert_eq!(value["model_id"], FLASH_MODEL);
    assert_eq!(value["language_code"], "vi");
    assert_eq!(value["apply_text_normalization"], "auto");
    assert!(value.get("voice_settings").is_none());
    assert!(value.get("previous_text").is_none());
    assert!(value.get("next_text").is_none());
}

#[test]
fn capability_gates_refuse_before_transport() {
    let fake = Arc::new(FakeTransport::new(Vec::new()));
    let multilingual = build_adapter(fake.clone(), MULTILINGUAL_MODEL, settings(), voices());
    let flash = build_adapter(fake.clone(), FLASH_MODEL, settings(), voices());
    let table = fixture_json("unsupported-language.json");

    for (adapter, key, expected_ok) in [
        (&multilingual, "multilingual_v2_supports", true),
        (&multilingual, "multilingual_v2_rejects", false),
        (&multilingual, "both_reject", false),
        (&flash, "flash_v2_5_supports", true),
        (&flash, "both_reject", false),
    ] {
        for tag in table[key].as_array().unwrap() {
            let tag = tag.as_str().unwrap();
            let language = if tag == "zh" {
                language("zh", "Hans")
            } else {
                language(tag, "Latn")
            };
            let request = build_request(
                adapter.config().clone(),
                "synthetic text",
                VOICE,
                language,
                output(),
                1_000_000,
            );
            let caps = adapter.capabilities(&request);
            let result = caps.check(&request);
            if expected_ok {
                assert!(result.is_ok(), "{key} should support {tag}");
            } else {
                assert!(
                    matches!(result, Err(Error::UnsupportedLanguage)),
                    "{key} must reject {tag}"
                );
                assert!(matches!(
                    SynthesisConsent::grant(
                        &request,
                        &caps,
                        adapter.estimate(&request),
                        money(1_000_000)
                    ),
                    Err(Error::UnsupportedLanguage)
                ));
            }
        }
    }

    let voice_table = fixture_json("unsupported-voice.json");
    assert!(
        voice_table["allowed"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| id.as_str() == Some(VOICE))
    );
    let rejected = voice_table["rejected"].as_str().unwrap();
    let unknown_voice = build_request(
        multilingual.config().clone(),
        "synthetic text",
        rejected,
        language("en", "Latn"),
        output(),
        1_000_000,
    );
    let caps = multilingual.capabilities(&unknown_voice);
    assert!(matches!(
        caps.check(&unknown_voice),
        Err(Error::UnsupportedVoice)
    ));
    assert!(matches!(
        SynthesisConsent::grant(
            &unknown_voice,
            &caps,
            multilingual.estimate(&unknown_voice),
            money(1_000_000)
        ),
        Err(Error::UnsupportedVoice)
    ));

    for output in [
        SpeechOutput {
            format: SpeechFormat::Pcm16,
            sample_rate: 48_000,
        },
        SpeechOutput {
            format: SpeechFormat::WavPcm16,
            sample_rate: SAMPLE_RATE,
        },
    ] {
        let request = build_request(
            flash.config().clone(),
            "synthetic text",
            VOICE,
            language("vi", "Latn"),
            output,
            1_000_000,
        );
        let caps = flash.capabilities(&request);
        assert!(matches!(caps.check(&request), Err(Error::Unsupported)));
    }
    assert_eq!(fake.calls(), 0);
}

#[test]
fn cost_is_exact_per_scalar_with_minimum_one() {
    let fake = Arc::new(FakeTransport::new(Vec::new()));
    let flash = build_adapter(fake.clone(), FLASH_MODEL, settings(), voices());
    let multilingual = build_adapter(fake.clone(), MULTILINGUAL_MODEL, settings(), voices());
    let table = fixture_json("cost.json");
    assert_eq!(table["currency"].as_str(), Some("USD"));
    assert_eq!(
        table["flash_micros_per_character"].as_u64(),
        Some(FLASH_MICROS_PER_CHARACTER)
    );
    assert_eq!(
        table["multilingual_micros_per_character"].as_u64(),
        Some(MULTILINGUAL_MICROS_PER_CHARACTER)
    );
    assert_eq!(table["minimum_characters"].as_u64(), Some(1));
    for example in table["examples"].as_array().unwrap() {
        let model = example["model"].as_str().unwrap();
        let adapter = if model == FLASH_MODEL {
            &flash
        } else {
            &multilingual
        };
        let language = if model == FLASH_MODEL {
            language("vi", "Latn")
        } else {
            language("en", "Latn")
        };
        let request = build_request(
            adapter.config().clone(),
            example["text"].as_str().unwrap(),
            VOICE,
            language,
            output(),
            1_000_000,
        );
        let estimate = adapter.estimate(&request);
        if let Some(upper) = example["upper_bound_micros"].as_u64() {
            assert_eq!(estimate, SynthesisEstimate::UpperBound(money(upper)));
        } else {
            assert_eq!(
                estimate,
                SynthesisEstimate::Exact(money(example["micros"].as_u64().unwrap()))
            );
        }
    }

    let one = build_request(
        flash.config().clone(),
        "A",
        VOICE,
        language("vi", "Latn"),
        output(),
        1_000_000,
    );
    assert_eq!(
        flash.estimate(&one),
        SynthesisEstimate::Exact(money(FLASH_MICROS_PER_CHARACTER))
    );
    let one_multi = build_request(
        multilingual.config().clone(),
        "A",
        VOICE,
        language("en", "Latn"),
        output(),
        1_000_000,
    );
    assert_eq!(
        multilingual.estimate(&one_multi),
        SynthesisEstimate::Exact(money(MULTILINGUAL_MICROS_PER_CHARACTER))
    );

    let emoji = build_request(
        flash.config().clone(),
        "Xin chào 👩🏽‍💻",
        VOICE,
        language("vi", "Latn"),
        output(),
        1_000_000,
    );
    assert_eq!(emoji.characters(), 10);
    // Non-BMP text uses the larger UTF-16 count as a pre-call upper bound.
    assert_eq!(
        flash.estimate(&emoji),
        SynthesisEstimate::UpperBound(money(16 * FLASH_MICROS_PER_CHARACTER))
    );
    assert!(matches!(
        SynthesisConsent::grant(
            &emoji,
            &flash.capabilities(&emoji),
            flash.estimate(&emoji),
            money(13 * FLASH_MICROS_PER_CHARACTER)
        ),
        Err(Error::Cost)
    ));

    // A combining-mark payload is one grapheme but thousands of characters; the
    // quote must not undercount it as one.
    let combining = format!("q{}", "\u{0305}".repeat(10_000));
    let combining = build_request(
        flash.config().clone(),
        &combining,
        VOICE,
        language("vi", "Latn"),
        output(),
        1_000_000,
    );
    assert_eq!(combining.characters(), 1);
    assert_eq!(
        flash.estimate(&combining),
        SynthesisEstimate::Exact(money(10_001 * FLASH_MICROS_PER_CHARACTER))
    );

    // The provider header is authoritative when present, even if it differs.
    let fake = Arc::new(FakeTransport::new(vec![
        pcm_reply(pcm("success.wav"), Some(17)),
        pcm_reply(pcm("success.wav"), None),
    ]));
    let adapter = build_adapter(fake.clone(), FLASH_MODEL, settings(), voices());
    let request = build_request(
        adapter.config().clone(),
        "Xin chào 👩🏽‍💻",
        VOICE,
        language("vi", "Latn"),
        output(),
        1_000_000,
    );
    let approved = grant_consent(&adapter, &request, 1_000_000);
    assert!(matches!(
        synthesize(&adapter, &request, &approved, &Hooks::new()),
        Err(Error::Cost)
    ));

    let approved = SynthesisConsent::grant(
        &request,
        &adapter.capabilities(&request),
        adapter.estimate(&request),
        money(1_000_000),
    )
    .unwrap();
    let audio = synthesize(&adapter, &request, &approved, &Hooks::new()).unwrap();
    // Without the provider header, the conservative UTF-16 count is recorded.
    assert_eq!(audio.usage.billable_characters, Some(16));
    assert_eq!(
        audio.usage.actual_cost,
        money(16 * FLASH_MICROS_PER_CHARACTER)
    );
}

#[test]
fn settings_and_normalization_are_consent_bound() {
    let default = settings();
    assert!(
        default
            .settings_fingerprint()
            .contains("normalization=auto")
    );
    assert!(default.settings_fingerprint().contains("format=pcm_24000"));
    assert!(default.settings_fingerprint().contains("speed=none"));
    assert!(default.settings_fingerprint().contains("logging=default"));
    let table = fixture_json("normalization.json");
    assert_eq!(table["apply_text_normalization"].as_str(), Some("auto"));
    assert!(
        table["language_code_models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model.as_str() == Some(FLASH_MODEL))
    );
    assert!(
        table["language_code_unsupported_models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model.as_str() == Some(MULTILINGUAL_MODEL))
    );

    let fake = Arc::new(FakeTransport::new(vec![
        pcm_reply(pcm("success.wav"), Some(13)),
        pcm_reply(pcm("success.wav"), Some(13)),
    ]));
    let adapter = build_adapter(fake.clone(), MULTILINGUAL_MODEL, settings(), voices());
    let request = build_request(
        adapter.config().clone(),
        "Xin chào 👩🏽‍💻",
        VOICE,
        language("en", "Latn"),
        output(),
        1_000_000,
    );
    let approved = grant_consent(&adapter, &request, 1_000_000);
    synthesize(&adapter, &request, &approved, &Hooks::new()).unwrap();
    let calls = fake.take();
    let value = body(&calls[0]);
    assert_eq!(
        value["apply_text_normalization"],
        table["apply_text_normalization"]
    );
    assert!(value.get("language_code").is_none());
    let fingerprint = adapter.capabilities(&request).fingerprint;
    assert!(fingerprint.as_str().contains("normalization=auto"));
    for field in table["fingerprint_fields"].as_array().unwrap() {
        assert!(
            fingerprint.as_str().contains(field.as_str().unwrap()),
            "fingerprint is missing {}",
            field.as_str().unwrap()
        );
    }

    let speed = SpeechSpeed::new(0.85).unwrap();
    let speed_settings = ElevenLabsSpeechSettings {
        speed: Some(speed),
        logging: LoggingMode::Default,
    };
    let fake = Arc::new(FakeTransport::new(vec![pcm_reply(
        pcm("success.wav"),
        Some(13),
    )]));
    let adapter = build_adapter(fake.clone(), FLASH_MODEL, speed_settings, voices());
    let request = build_request(
        adapter.config().clone(),
        "Xin chào 👩🏽‍💻",
        VOICE,
        language("vi", "Latn"),
        output(),
        1_000_000,
    );
    let approved = grant_consent(&adapter, &request, 1_000_000);
    synthesize(&adapter, &request, &approved, &Hooks::new()).unwrap();
    let calls = fake.take();
    let value = body(&calls[0]);
    assert_eq!(value["voice_settings"]["speed"], 0.85);
    assert!(
        adapter
            .capabilities(&request)
            .fingerprint
            .as_str()
            .contains("speed=0.85")
    );
}

#[test]
fn zero_retention_is_requested_but_never_claimed() {
    let zrm = ElevenLabsSpeechSettings {
        speed: None,
        logging: LoggingMode::ZeroRetentionRequested,
    };
    let fake = Arc::new(FakeTransport::new(vec![pcm_reply(
        pcm("success.wav"),
        Some(13),
    )]));
    let adapter = build_adapter(fake.clone(), FLASH_MODEL, zrm, voices());
    let request = build_request(
        adapter.config().clone(),
        "Xin chào 👩🏽‍💻",
        VOICE,
        language("vi", "Latn"),
        output(),
        1_000_000,
    );
    let disclosure = adapter.disclosure(&request).unwrap();
    assert!(disclosure.logging.as_str().contains("Enterprise"));
    assert!(
        disclosure
            .policy
            .retention
            .as_str()
            .contains("does not verify")
    );
    assert_eq!(disclosure.model.as_str(), FLASH_MODEL);
    assert_eq!(disclosure.voice.as_str(), VOICE);
    assert_eq!(disclosure.billable_characters, 16);
    assert_eq!(
        disclosure.estimate,
        SynthesisEstimate::UpperBound(money(16 * FLASH_MICROS_PER_CHARACTER))
    );
    assert_eq!(disclosure.speed, None);
    let approved = grant_consent(&adapter, &request, 1_000_000);
    synthesize(&adapter, &request, &approved, &Hooks::new()).unwrap();
    let calls = fake.take();
    assert!(calls[0].url.ends_with("&enable_logging=false"));

    let default = build_adapter(
        Arc::new(FakeTransport::new(vec![pcm_reply(
            pcm("success.wav"),
            Some(13),
        )])),
        FLASH_MODEL,
        settings(),
        voices(),
    );
    let disclosure = default
        .disclosure(&request_with(default.config().clone()))
        .unwrap();
    assert!(
        !disclosure
            .policy
            .retention
            .as_str()
            .contains("zero retention")
    );
    assert!(!disclosure.logging.as_str().contains("Enterprise"));
}

fn request_with(provider: ProviderConfig) -> SpeechRequest {
    build_request(
        provider,
        "Xin chào 👩🏽‍💻",
        VOICE,
        language("vi", "Latn"),
        output(),
        1_000_000,
    )
}

#[test]
fn long_text_splits_at_boundaries_without_losing_text() {
    let fixture = fixture_json("split.json");
    let limit = fixture["model_limit"].as_u64().unwrap() as usize;
    assert_eq!(limit, 10_000);
    let cue = format!(
        "{}\n{}",
        "a".repeat(fixture["prefix_count"].as_u64().unwrap() as usize),
        "b".repeat(fixture["suffix_count"].as_u64().unwrap() as usize)
    );
    let cases = [
        (
            cue,
            vec![format!("{}\n", "a".repeat(9_000)), "b".repeat(2_000)],
        ),
        (
            "x".repeat(25_001),
            vec!["x".repeat(10_000), "x".repeat(10_000), "x".repeat(5_001)],
        ),
    ];

    for (text, expected) in cases {
        let replies = expected
            .iter()
            .map(|chunk| pcm_reply(pcm("success.wav"), Some(chunk.chars().count() as u64)))
            .collect();
        let fake = Arc::new(FakeTransport::new(replies));
        let adapter = build_adapter(fake.clone(), MULTILINGUAL_MODEL, settings(), voices());
        let request = build_request(
            adapter.config().clone(),
            &text,
            VOICE,
            language("en", "Latn"),
            output(),
            1_000_000,
        );
        let approved = grant_consent(&adapter, &request, 3_000_000);
        let audio = synthesize(&adapter, &request, &approved, &Hooks::new()).unwrap();
        assert_eq!(audio.samples.len(), expected.len() * 6_000);
        assert_eq!(audio.duration_micros, expected.len() as u64 * 250_000);
        assert_eq!(
            audio.usage.billable_characters,
            Some(text.chars().count() as u64)
        );

        let calls = fake.take();
        assert_eq!(calls.len(), expected.len());
        let actual = calls
            .iter()
            .map(|call| body(call)["text"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
        assert_eq!(actual.concat(), text);
        for (call, chunk) in calls.iter().zip(&expected) {
            let value = body(call);
            assert_eq!(value["model_id"], MULTILINGUAL_MODEL);
            assert!(value.get("language_code").is_none());
            assert!(chunk.chars().count() <= limit);
        }
        let first = body(&calls[0]);
        assert!(first.get("previous_text").is_none());
        assert!(body(&calls[calls.len() - 1]).get("next_text").is_none());
        for call in &calls[1..] {
            assert!(body(call).get("previous_text").is_some());
        }
        for call in &calls[..calls.len() - 1] {
            assert!(body(call).get("next_text").is_some());
        }
        if expected.len() == 2 {
            assert_eq!(first["next_text"].as_str().unwrap(), "b".repeat(1_000));
            assert_eq!(
                body(&calls[1])["previous_text"].as_str().unwrap(),
                format!("{}\n", "a".repeat(999))
            );
        }
    }
}

#[test]
fn errors_are_typed_and_never_retried() {
    let cases = [
        ("error-auth.json", "auth"),
        ("error-quota.json", "quota"),
        ("error-rate-limit.json", "rate"),
        ("error-content-policy.json", "content"),
    ];
    for (file, kind) in cases {
        let fake = Arc::new(FakeTransport::new(vec![error_reply(file)]));
        let adapter = build_adapter(fake.clone(), FLASH_MODEL, settings(), voices());
        let request = build_request(
            adapter.config().clone(),
            "synthetic text",
            VOICE,
            language("vi", "Latn"),
            output(),
            1_000_000,
        );
        let approved = grant_consent(&adapter, &request, 1_000_000);
        let error = synthesize(&adapter, &request, &approved, &Hooks::new()).unwrap_err();
        match kind {
            "auth" => assert!(matches!(error, Error::Auth)),
            "quota" => assert!(matches!(error, Error::Quota)),
            "rate" => assert!(matches!(
                error,
                Error::RateLimit {
                    retry_after_seconds: Some(7)
                }
            )),
            _ => assert!(matches!(error, Error::ContentPolicy)),
        }
        assert_eq!(fake.calls(), 1);
    }

    for (status, expected) in [
        (400, "payload"),
        (404, "voice"),
        (408, "timeout"),
        (409, "invalid"),
        (413, "payload"),
        (500, "server"),
        (302, "invalid"),
    ] {
        let fake = Arc::new(FakeTransport::new(vec![Reply::Response {
            status,
            headers: Vec::new(),
            body: b"{}".to_vec(),
        }]));
        let adapter = build_adapter(fake.clone(), FLASH_MODEL, settings(), voices());
        let request = build_request(
            adapter.config().clone(),
            "synthetic text",
            VOICE,
            language("vi", "Latn"),
            output(),
            1_000_000,
        );
        let approved = grant_consent(&adapter, &request, 1_000_000);
        let error = synthesize(&adapter, &request, &approved, &Hooks::new()).unwrap_err();
        match expected {
            "payload" => assert!(matches!(error, Error::Payload)),
            "voice" => assert!(matches!(error, Error::UnsupportedVoice)),
            "timeout" => assert!(matches!(error, Error::Timeout)),
            "invalid" => assert!(matches!(error, Error::Invalid(_))),
            _ => assert!(matches!(error, Error::Server)),
        }
        assert_eq!(fake.calls(), 1);
    }
}

#[test]
fn malformed_and_oversized_audio_is_rejected() {
    let cases = [
        (Vec::new(), Some(1)),
        (vec![0, 1, 2], Some(1)),
        (vec![0; 48_002], Some(1)),
    ];
    for (body, cost) in cases {
        let fake = Arc::new(FakeTransport::new(vec![pcm_reply(body, cost)]));
        let adapter = build_adapter(fake.clone(), FLASH_MODEL, settings(), voices());
        let request = build_request(
            adapter.config().clone(),
            "synthetic text",
            VOICE,
            language("vi", "Latn"),
            output(),
            1_000_000,
        );
        let approved = grant_consent(&adapter, &request, 1_000_000);
        let error = synthesize(&adapter, &request, &approved, &Hooks::new()).unwrap_err();
        assert!(matches!(error, Error::Payload | Error::OutputRejected));
        assert_eq!(fake.calls(), 1);
    }

    // Two individually admitted chunks can still exceed the combined duration.
    let fake = Arc::new(FakeTransport::new(vec![
        pcm_reply(vec![0; 4_800], Some(10_000)),
        pcm_reply(vec![0; 4_800], Some(10_000)),
    ]));
    let adapter = build_adapter(fake.clone(), MULTILINGUAL_MODEL, settings(), voices());
    let text = format!("{}\n{}", "a".repeat(9_000), "b".repeat(2_000));
    let request = build_request(
        adapter.config().clone(),
        &text,
        VOICE,
        language("en", "Latn"),
        output(),
        150_000,
    );
    let approved = grant_consent(&adapter, &request, 1_000_000);
    assert!(matches!(
        synthesize(&adapter, &request, &approved, &Hooks::new()),
        Err(Error::OutputRejected)
    ));

    // Only a complete 200 body is success. Other 2xx statuses are partial or
    // incomplete, and a declared HTTP body length must match the received bytes.
    let body = pcm("success.wav");
    let mut replies = Vec::new();
    for status in [201, 204, 206] {
        let body = if status == 204 {
            Vec::new()
        } else {
            body.clone()
        };
        replies.push(Reply::Response {
            status,
            headers: vec![("content-length".into(), body.len().to_string())],
            body,
        });
    }
    replies.push(Reply::Response {
        status: 200,
        headers: vec![
            ("content-length".into(), (body.len() - 1).to_string()),
            ("character-cost".into(), "13".into()),
        ],
        body: body.clone(),
    });
    for reply in replies {
        let fake = Arc::new(FakeTransport::new(vec![reply]));
        let adapter = build_adapter(fake.clone(), FLASH_MODEL, settings(), voices());
        let request = build_request(
            adapter.config().clone(),
            "synthetic text",
            VOICE,
            language("vi", "Latn"),
            output(),
            1_000_000,
        );
        let approved = grant_consent(&adapter, &request, 1_000_000);
        assert!(matches!(
            synthesize(&adapter, &request, &approved, &Hooks::new()),
            Err(Error::OutputRejected)
        ));
        assert_eq!(fake.calls(), 1);
    }
}

#[test]
fn synthesis_failures_are_typed_without_partial_audio() {
    let fake = Arc::new(FakeTransport::new(vec![Reply::Failure(Error::Network)]));
    let adapter = build_adapter(fake.clone(), FLASH_MODEL, settings(), voices());
    let request = build_request(
        adapter.config().clone(),
        "synthetic text",
        VOICE,
        language("vi", "Latn"),
        output(),
        1_000_000,
    );
    let approved = grant_consent(&adapter, &request, 1_000_000);
    assert!(matches!(
        synthesize(&adapter, &request, &approved, &Hooks::new()),
        Err(Error::Network)
    ));
    assert_eq!(fake.calls(), 1);

    let fake = Arc::new(FakeTransport::new(vec![pcm_reply(
        pcm("success.wav"),
        Some(13),
    )]));
    let adapter = build_adapter(fake.clone(), FLASH_MODEL, settings(), voices());
    let flag = Arc::new(AtomicBool::new(true));
    let hooks = Hooks::shared(flag);
    assert!(matches!(
        synthesize(&adapter, &request, &approved, &hooks),
        Err(Error::Cancelled)
    ));
    assert_eq!(fake.calls(), 0);

    // A later split chunk failing never returns the earlier audio.
    let split = format!("{}\n{}", "a".repeat(9_000), "b".repeat(2_000));
    let fake = Arc::new(FakeTransport::new(vec![
        pcm_reply(pcm("success.wav"), Some(9_001)),
        Reply::Failure(Error::Server),
    ]));
    let adapter = build_adapter(fake.clone(), MULTILINGUAL_MODEL, settings(), voices());
    let split_request = build_request(
        adapter.config().clone(),
        &split,
        VOICE,
        language("en", "Latn"),
        output(),
        1_000_000,
    );
    let approved = grant_consent(&adapter, &split_request, 1_000_000);
    assert!(matches!(
        synthesize(&adapter, &split_request, &approved, &Hooks::new()),
        Err(Error::Server)
    ));
    assert_eq!(fake.calls(), 2);

    // Cancellation between chunks stops before issuing the next request.
    let cancellation = fixture_json("cancellation.json");
    assert_eq!(cancellation["chunks"].as_u64(), Some(2));
    let expected_calls = cancellation["expected_calls_before_cancel"]
        .as_u64()
        .unwrap() as usize;
    let flag = Arc::new(AtomicBool::new(false));
    let fake = Arc::new(FakeTransport::cancelling(
        vec![pcm_reply(pcm("success.wav"), Some(9_001))],
        flag.clone(),
        expected_calls,
    ));
    let adapter = build_adapter(fake.clone(), MULTILINGUAL_MODEL, settings(), voices());
    let split_request = build_request(
        adapter.config().clone(),
        &split,
        VOICE,
        language("en", "Latn"),
        output(),
        1_000_000,
    );
    let approved = grant_consent(&adapter, &split_request, 1_000_000);
    let hooks = Hooks::shared(flag);
    assert!(matches!(
        synthesize(&adapter, &split_request, &approved, &hooks),
        Err(Error::Cancelled)
    ));
    assert_eq!(fake.calls(), expected_calls);
}

#[test]
fn unsupported_configuration_and_voices_are_refused() {
    let fake = Arc::new(FakeTransport::new(Vec::new()));
    let key = ApiKey::new("test-key-not-real").unwrap();

    for model in ["eleven_v3", "eleven_turbo_v2_5"] {
        let config = provider_config(model, settings(), BASE_URL);
        assert!(matches!(
            ElevenLabsSpeech::new(config, key.clone(), fake.clone(), settings(), voices()),
            Err(Error::Unsupported)
        ));
    }

    for base_url in [
        "https://api.elevenlabs.io/v1/",
        "https://api.elevenlabs.io/v1/other",
        "https://lookalike.example/v1",
    ] {
        let provider = provider_config(FLASH_MODEL, settings(), base_url);
        let result =
            ElevenLabsSpeech::new(provider, key.clone(), fake.clone(), settings(), voices());
        if base_url == "https://api.elevenlabs.io/v1/" {
            assert!(result.is_ok());
        } else {
            assert!(matches!(result, Err(Error::Unsupported)));
        }
    }

    for invalid in [0.69, 1.21, f64::NAN, f64::INFINITY] {
        assert!(matches!(SpeechSpeed::new(invalid), Err(Error::Invalid(_))));
    }
    let wrong_settings = ProviderConfig::new(
        "elevenlabs-speech".into(),
        FLASH_MODEL.into(),
        "elevenlabs-speech-v1;format=pcm_24000;normalization=off;speed=none;logging=default".into(),
        Location::Remote {
            backend: "elevenlabs".into(),
            base_url: BASE_URL.into(),
            auth_ref: "keychain:vibecut/elevenlabs".into(),
        },
    )
    .unwrap();
    assert!(matches!(
        ElevenLabsSpeech::new(
            wrong_settings,
            key.clone(),
            fake.clone(),
            settings(),
            voices()
        ),
        Err(Error::Unsupported)
    ));

    let local = ProviderConfig::new(
        "elevenlabs-speech".into(),
        FLASH_MODEL.into(),
        settings().settings_fingerprint().into(),
        Location::Local {
            backend: "elevenlabs".into(),
            model_ref: "local".into(),
        },
    )
    .unwrap();
    assert!(matches!(
        ElevenLabsSpeech::new(local, key.clone(), fake.clone(), settings(), voices()),
        Err(Error::Invalid(_))
    ));

    for stock in [Vec::new(), vec![voices()[0].clone(), voices()[0].clone()]] {
        assert!(matches!(
            ElevenLabsSpeech::new(
                provider_config(FLASH_MODEL, settings(), BASE_URL),
                key.clone(),
                fake.clone(),
                settings(),
                stock
            ),
            Err(Error::Invalid(_))
        ));
    }

    for id in ["", "has/slash", "has space"] {
        let stock = vec![SpeechVoice {
            id: NfcText::from(id),
            category: VoiceCategory::BuiltIn,
        }];
        assert!(matches!(
            ElevenLabsSpeech::new(
                provider_config(FLASH_MODEL, settings(), BASE_URL),
                key.clone(),
                fake.clone(),
                settings(),
                stock
            ),
            Err(Error::Invalid(_))
        ));
    }
}

#[test]
#[ignore = "requires VIBECUT_LIVE_AI=1, ELEVENLABS_API_KEY and a premade/stock VIBECUT_ELEVENLABS_VOICE_ID"]
fn live_elevenlabs_speech_uses_production_transport() {
    if std::env::var("VIBECUT_LIVE_AI").as_deref() != Ok("1") {
        eprintln!("live test skipped: set VIBECUT_LIVE_AI=1 to run");
        return;
    }
    let key = std::env::var("ELEVENLABS_API_KEY").expect("ELEVENLABS_API_KEY");
    let voice = std::env::var("VIBECUT_ELEVENLABS_VOICE_ID")
        .expect("VIBECUT_ELEVENLABS_VOICE_ID (a premade/stock voice)");
    let cases = [
        (
            "vi",
            "Latn",
            FLASH_MODEL,
            "Xin chào, đây là một bài kiểm tra giọng nói.",
        ),
        (
            "en",
            "Latn",
            MULTILINGUAL_MODEL,
            "Hello, this is a synthetic speech test.",
        ),
        (
            "zh",
            "Hans",
            MULTILINGUAL_MODEL,
            "你好，这是一次合成语音测试。",
        ),
    ];
    for (tag, script, model, text) in cases {
        let settings = settings();
        let config = provider_config(model, settings, BASE_URL);
        let adapter = ElevenLabsSpeech::new(
            config,
            ApiKey::new(key.clone()).unwrap(),
            Arc::new(UreqTransport::new()),
            settings,
            vec![SpeechVoice {
                id: NfcText::from(voice.as_str()),
                category: VoiceCategory::Stock,
            }],
        )
        .unwrap();
        let request = build_request(
            adapter.config().clone(),
            text,
            &voice,
            language(tag, script),
            output(),
            15_000_000,
        );
        let estimate = adapter.estimate(&request);
        let SynthesisEstimate::Exact(cost) = &estimate else {
            panic!("live estimate must be exact");
        };
        assert!(cost.micros() <= 10_000, "bounded live cost");
        let approved = SynthesisConsent::grant(
            &request,
            &adapter.capabilities(&request),
            estimate,
            money(10_000),
        )
        .unwrap();
        let audio = synthesize(&adapter, &request, &approved, &Hooks::new()).unwrap();
        assert!(!audio.samples.is_empty());
        assert!(audio.samples.iter().all(|sample| sample.is_finite()));
        assert!(audio.duration_micros > 0);
    }
}
