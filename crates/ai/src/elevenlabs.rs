//! ElevenLabs Scribe file transcription: consent-gated HTTPS transport, word-level
//! token spans mapped through captions, and typed failures.
use crate::contracts::checkpoint;
use crate::transport::{
    ApiKey, HttpRequest, HttpResponse, HttpTransport, MAX_RESPONSE_BYTES, Multipart, retry_after,
    wav, wav_bytes,
};
use crate::{
    AudioWindow, Capabilities, CostEstimate, Error, Evidence, ExecutionHooks, Location, Money,
    Operation, Provenance, ProviderConfig, RawToken, RecognitionSegment, Request, SampleRange,
    Scope, Stage, Support, TimingUnit, Transcriber,
};
use captions::{LanguageTag, NfcText};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

pub const MODEL: &str = "scribe_v2";
pub const BACKEND: &str = "elevenlabs";
pub const BASE_URL: &str = "https://api.elevenlabs.io/v1";
pub const ENDPOINT: &str = "https://api.elevenlabs.io/v1/speech-to-text";
/// Documented standard-mode file ceiling; duration is separately capped at 10 h.
pub const MAX_UPLOAD_BYTES: u64 = 3_000_000_000;
pub const MAX_DURATION_MICROS: u64 = 10 * 60 * 60 * 1_000_000;
/// Published list price, integer micro-USD per audio hour ($0.22/h).
const PRICE_MICROS_PER_HOUR: u64 = 220_000;
const MODE: &str = "file";
const LANGUAGES: [&str; 3] = ["vi", "en", "zh"];
pub const SETTINGS_FINGERPRINT: &str = "elevenlabs-transport-v1;model=scribe_v2;format=json;wav=pcm16;timestamps=word;languages=vi,en,zh;logging=default";
const DATA_USE_URL: &str =
    "https://elevenlabs.io/docs/help-center/legal/is-my-data-used-to-improve-eleven-labs-ai-models";
const ZERO_RETENTION_URL: &str =
    "https://elevenlabs.io/docs/eleven-api/resources/zero-retention-mode";
const DOCS_URL: &str = "https://elevenlabs.io/docs/capabilities/speech-to-text";
const PRICING_URL: &str = "https://elevenlabs.io/pricing/api?price.section=speech_to_text";
const TRAINING_DISCLOSURE: &str = "ElevenLabs may use submitted audio and text to improve its models by default; any account can opt out under Terms & privacy > Data use, and Enterprise accounts are excluded by default. This adapter cannot read the current account's toggle (checked 2026-11-04).";
const RETENTION_DISCLOSURE: &str = "ElevenLabs retains speech-to-text audio and text by default and offers deletion/history controls; per-account retention settings are not visible to this adapter (checked 2026-11-04).";
const ZERO_RETENTION_DISCLOSURE: &str = "Zero Retention Mode is enterprise-only and requested per request with enable_logging=false; this adapter uses default logging and does not assume Zero Retention Mode eligibility.";

/// What the composition root must show before granting upload consent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UploadDisclosure {
    pub operation: Operation,
    pub provider: NfcText,
    pub model: NfcText,
    pub endpoint: NfcText,
    pub language: LanguageTag,
    pub format: NfcText,
    pub audio_micros: u64,
    pub upload_bytes: u64,
    pub estimate: CostEstimate,
    pub training: NfcText,
    pub retention: NfcText,
    pub zero_retention: NfcText,
    pub data_use_url: NfcText,
    pub zero_retention_url: NfcText,
    pub docs_url: NfcText,
    pub pricing_url: NfcText,
}

pub struct ElevenLabsTranscriber {
    config: ProviderConfig,
    key: ApiKey,
    transport: Arc<dyn HttpTransport>,
    languages: Vec<LanguageTag>,
}
impl ElevenLabsTranscriber {
    /// Admits only `scribe_v2`, backend `elevenlabs` and the official HTTPS base URL.
    pub fn new(
        config: ProviderConfig,
        key: ApiKey,
        transport: Arc<dyn HttpTransport>,
    ) -> Result<Self, Error> {
        if config.model().as_str() != MODEL {
            return Err(Error::Unsupported);
        }
        match config.location() {
            Location::Remote {
                backend, base_url, ..
            } => {
                if backend.as_str() != BACKEND {
                    return Err(Error::Unsupported);
                }
                official_base_url(base_url.as_str())?;
            }
            Location::Local { .. } => {
                return Err(Error::Invalid("ElevenLabs requires remote config"));
            }
        }
        let languages = LANGUAGES
            .iter()
            .map(|tag| LanguageTag::parse(tag).map_err(Error::Captions))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            config,
            key,
            transport,
            languages,
        })
    }

    pub fn settings_fingerprint() -> &'static str {
        SETTINGS_FINGERPRINT
    }

    /// Current account-aware retention/training disclosure with source links.
    pub fn disclosure(&self, scope: &Scope) -> Result<UploadDisclosure, Error> {
        self.validate_scope(scope)?;
        let audio_micros = duration_micros(scope)?;
        let upload_bytes = encoded_bytes(scope)?;
        Ok(UploadDisclosure {
            operation: Operation::Transcribe,
            provider: self.config.id().clone(),
            model: self.config.model().clone(),
            endpoint: NfcText::from(ENDPOINT),
            language: scope.source_language.clone(),
            format: NfcText::from(format!(
                "audio/wav; mono 16-bit PCM at {} Hz",
                scope.sample_rate
            )),
            audio_micros,
            upload_bytes,
            estimate: self.estimate(scope),
            training: NfcText::from(TRAINING_DISCLOSURE),
            retention: NfcText::from(RETENTION_DISCLOSURE),
            zero_retention: NfcText::from(ZERO_RETENTION_DISCLOSURE),
            data_use_url: NfcText::from(DATA_USE_URL),
            zero_retention_url: NfcText::from(ZERO_RETENTION_URL),
            docs_url: NfcText::from(DOCS_URL),
            pricing_url: NfcText::from(PRICING_URL),
        })
    }

    fn validate_scope(&self, scope: &Scope) -> Result<(), Error> {
        if scope.provider != self.config {
            return Err(Error::Unsupported);
        }
        if !scope.texts.is_empty() {
            return Err(Error::Invalid("transcription text payload"));
        }
        self.capabilities(scope).check(scope)
    }
}
impl fmt::Debug for ElevenLabsTranscriber {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ElevenLabsTranscriber")
            .field("config", &self.config)
            .field("key", &self.key)
            .field("transport", &"[transport]")
            .field("languages", &self.languages)
            .finish()
    }
}
impl Transcriber for ElevenLabsTranscriber {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    fn capabilities(&self, _scope: &Scope) -> Capabilities {
        Capabilities {
            model: self.config.model().clone(),
            mode: NfcText::from(MODE),
            operation: Operation::Transcribe,
            languages: self.languages.clone(),
            target_languages: Vec::new(),
            timing: BTreeSet::from([TimingUnit::Segment, TimingUnit::Token]),
            diarization: Support::Unsupported,
            live_audio_input: Support::Unsupported,
            file_result_stream: Support::Unsupported,
            max_upload_bytes: Some(MAX_UPLOAD_BYTES),
            max_duration_micros: Some(MAX_DURATION_MICROS),
            evidence: Evidence::FixtureVerified,
            revision: NfcText::from("scribe_v2-2026-11-04"),
        }
    }

    fn estimate(&self, scope: &Scope) -> CostEstimate {
        let Some(seconds) = rounded_up_seconds(scope) else {
            return CostEstimate::Unknown;
        };
        let Some(micros) = seconds
            .checked_mul(u128::from(PRICE_MICROS_PER_HOUR))
            .and_then(|value| value.checked_add(3_599))
            .map(|value| value / 3_600)
        else {
            return CostEstimate::Unknown;
        };
        let Ok(micros) = u64::try_from(micros) else {
            return CostEstimate::Unknown;
        };
        match Money::new("USD", micros) {
            Ok(money) => CostEstimate::UpperBound(money),
            Err(_) => CostEstimate::Unknown,
        }
    }

    fn transcribe(
        &self,
        audio: AudioWindow<'_>,
        request: &Request<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<RecognitionSegment>, Error> {
        checkpoint(hooks, Stage::Transcribe)?;
        let upload = self.authorize(audio, request)?;
        let outbound = build_request(&upload, audio)?;
        checkpoint(hooks, Stage::Transcribe)?;
        let response = self.transport.send(outbound)?;
        checkpoint(hooks, Stage::Transcribe)?;
        let segments = parse_transcription(&response, audio.range, audio.sample_rate)?;
        checkpoint(hooks, Stage::Transcribe)?;
        Ok(segments)
    }
}

struct AuthorizedUpload<'a> {
    scope: &'a Scope,
    key: &'a ApiKey,
}

impl ElevenLabsTranscriber {
    fn authorize<'a>(
        &'a self,
        audio: AudioWindow<'_>,
        request: &'a Request<'_>,
    ) -> Result<AuthorizedUpload<'a>, Error> {
        let scope = request.scope;
        self.validate_scope(scope)?;
        audio.validate(scope)?;
        if encoded_bytes(scope)? > scope.max_bytes {
            return Err(Error::Payload);
        }
        request.prepare(
            &self.capabilities(scope),
            &self.estimate(scope),
            Operation::Transcribe,
        )?;
        Ok(AuthorizedUpload {
            scope,
            key: &self.key,
        })
    }
}

fn official_base_url(base_url: &str) -> Result<(), Error> {
    if base_url.trim_end_matches('/') == BASE_URL {
        Ok(())
    } else {
        // The retention/training disclosure is account and endpoint specific; never
        // apply it to a proxy, lookalike host or unsupported version path.
        Err(Error::Unsupported)
    }
}

fn duration_micros(scope: &Scope) -> Result<u64, Error> {
    let micros = u128::from(scope.source_samples.len())
        .checked_mul(1_000_000)
        .ok_or(Error::Payload)?
        / u128::from(scope.sample_rate);
    u64::try_from(micros).map_err(|_| Error::Payload)
}

fn rounded_up_seconds(scope: &Scope) -> Option<u128> {
    let samples = u128::from(scope.source_samples.len());
    let rate = u128::from(scope.sample_rate);
    if rate == 0 {
        return None;
    }
    Some(samples.checked_add(rate - 1)? / rate)
}

fn encoded_bytes(scope: &Scope) -> Result<u64, Error> {
    let bytes = wav_bytes(scope.source_samples.len())?;
    if bytes > MAX_UPLOAD_BYTES {
        return Err(Error::Payload);
    }
    Ok(bytes)
}

fn build_request(
    upload: &AuthorizedUpload<'_>,
    audio: AudioWindow<'_>,
) -> Result<HttpRequest, Error> {
    let audio = wav(&audio)?;
    if u64::try_from(audio.len()).map_err(|_| Error::Payload)? > MAX_UPLOAD_BYTES {
        return Err(Error::Payload);
    }
    let mut form = Multipart::new(&audio)?;
    form.field("model_id", MODEL);
    form.field("language_code", upload.scope.source_language.as_str());
    form.field("timestamps_granularity", "word");
    form.file("audio.wav", "audio/wav", &audio);
    let (content_type, body) = form.finish();
    Ok(HttpRequest {
        method: "POST",
        url: ENDPOINT.into(),
        headers: vec![
            ("xi-api-key".into(), upload.key.expose().to_owned()),
            ("Content-Type".into(), content_type),
            ("Accept".into(), "application/json".into()),
        ],
        body,
    })
}

#[derive(Deserialize)]
struct Transcription {
    text: String,
    #[serde(default)]
    words: Vec<Word>,
}
#[derive(Deserialize)]
struct Word {
    text: String,
    start: Option<f64>,
    end: Option<f64>,
    #[serde(rename = "type", default)]
    kind: Option<String>,
}

struct Group {
    bytes: Vec<u8>,
    start: Option<f64>,
    end: Option<f64>,
}

fn parse_transcription(
    response: &HttpResponse,
    window: SampleRange,
    sample_rate: u32,
) -> Result<Vec<RecognitionSegment>, Error> {
    if response.body.len() > MAX_RESPONSE_BYTES {
        return Err(Error::Payload);
    }
    if !(200..=299).contains(&response.status) {
        return Err(status_error(response));
    }
    let parsed: Transcription =
        serde_json::from_slice(&response.body).map_err(|_| Error::Invalid("provider response"))?;
    if parsed.text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut groups: Vec<Group> = Vec::new();
    for word in &parsed.words {
        if word.text.is_empty() {
            continue;
        }
        if matches!(word.kind.as_deref(), Some("word" | "audio_event")) || groups.is_empty() {
            groups.push(Group {
                bytes: word.text.clone().into_bytes(),
                start: word.start,
                end: word.end,
            });
        } else if let Some(group) = groups.last_mut() {
            // Spacing stays with the preceding token, preserving the exact text
            // without importing overlapping spacing timings. Audio events keep
            // their own native timing instead of inheriting the preceding word.
            group.bytes.extend_from_slice(word.text.as_bytes());
        }
    }
    if groups.is_empty() {
        groups.push(Group {
            bytes: parsed.text.clone().into_bytes(),
            start: None,
            end: None,
        });
    }
    let raw = groups
        .iter()
        .flat_map(|group| group.bytes.iter().copied())
        .collect::<Vec<_>>();
    let raw = String::from_utf8(raw).map_err(|_| Error::Invalid("provider response text"))?;
    let text = NfcText::from(raw);
    if text != NfcText::from(parsed.text) {
        return Err(Error::Invalid("provider response text mismatch"));
    }
    let mut tokens = Vec::with_capacity(groups.len());
    let mut timed = Vec::new();
    for group in groups {
        let range = match (group.start, group.end) {
            (Some(start), Some(end)) => sample_range(start, end, window, sample_rate)?,
            _ => None,
        };
        if let Some(range) = range {
            timed.push(range);
        }
        tokens.push(RawToken {
            bytes: group.bytes,
            range,
            provenance: Provenance::Native,
            confidence: None,
        });
    }
    let range = match (timed.first(), timed.last()) {
        (Some(first), Some(last)) => Some(SampleRange::new(first.start(), last.end())?),
        _ => None,
    };
    Ok(vec![RecognitionSegment {
        id: NfcText::from(format!("elevenlabs-{}-{}-0", window.start(), window.end())),
        text,
        range,
        timing: Vec::new(),
        raw_tokens: tokens,
    }])
}

fn sample_range(
    start: f64,
    end: f64,
    window: SampleRange,
    sample_rate: u32,
) -> Result<Option<SampleRange>, Error> {
    let start = sample_at(start, window, sample_rate)?;
    let end = sample_at(end, window, sample_rate)?;
    if end <= start {
        // A zero/negative provider span stays explicitly untimed rather than inventing
        // a duration; the shared token layer accepts a missing range.
        return Ok(None);
    }
    Ok(Some(SampleRange::new(start, end)?))
}

fn sample_at(seconds: f64, window: SampleRange, sample_rate: u32) -> Result<u64, Error> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(Error::Invalid("provider timing"));
    }
    let scaled = seconds * f64::from(sample_rate);
    if !scaled.is_finite() || scaled > u64::MAX as f64 {
        return Err(Error::Payload);
    }
    let relative = scaled.round();
    if relative > (window.len() as f64) + 1.0 {
        return Err(Error::Invalid("provider timing outside audio"));
    }
    let relative = relative.max(0.0) as u64;
    Ok(window.start().saturating_add(relative).min(window.end()))
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    detail: Option<ErrorDetail>,
}
#[derive(Deserialize)]
struct ErrorDetail {
    code: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
}

fn status_error(response: &HttpResponse) -> Error {
    match response.status {
        401 | 403 => Error::Auth,
        400 if auth_error(&response.body) => Error::Auth,
        400 | 422 if payload_error(&response.body) => Error::Payload,
        402 => Error::Quota,
        413 => Error::Payload,
        429 => Error::RateLimit {
            retry_after_seconds: retry_after(&response.headers),
        },
        500..=599 => Error::Server,
        300..=399 => Error::Invalid("provider redirect"),
        400 | 404 | 409 | 422 => Error::Invalid("provider rejected request"),
        _ => Error::Server,
    }
}

fn detail_matches(body: &[u8], needles: &[&str]) -> bool {
    let Ok(envelope) = serde_json::from_slice::<ErrorEnvelope>(body) else {
        return false;
    };
    envelope.detail.is_some_and(|detail| {
        [detail.code, detail.kind]
            .into_iter()
            .flatten()
            .any(|value| {
                let value = value.to_ascii_lowercase();
                needles.iter().any(|needle| value.contains(needle))
            })
    })
}

fn auth_error(body: &[u8]) -> bool {
    detail_matches(body, &["invalid_api_key", "authentication"])
}

fn payload_error(body: &[u8]) -> bool {
    detail_matches(body, &["file_too_large", "too_large", "payload"])
}
