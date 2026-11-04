//! OpenAI providers: consent-gated HTTPS transport, typed failures, text-only file
//! recognition, contextual translation and Chat Completions speech synthesis.
mod speech;
mod translation;
use crate::contracts::checkpoint;
use crate::{
    AudioWindow, Capabilities, CostEstimate, Error, Evidence, ExecutionHooks, Location, Money,
    Operation, ProviderConfig, RecognitionSegment, Request, SampleRange, Scope, Stage, Support,
    Transcriber,
};
use captions::{LanguageTag, NfcText};
use serde::Deserialize;
pub use speech::*;
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;
pub use translation::*;

use crate::transport::{
    ApiKey, HttpRequest, HttpResponse, HttpTransport, MAX_RESPONSE_BYTES, Multipart, retry_after,
    wav, wav_bytes,
};

/// The only production model admitted here. Deprecated transcription models are refused.
pub const MODEL: &str = "gpt-transcribe";
/// Provider backend name expected in the tagged remote configuration.
pub const BACKEND: &str = "openai";
/// Current documented file limit for `/v1/audio/transcriptions` (25 MB decimal).
pub const MAX_UPLOAD_BYTES: u64 = 25_000_000;
/// The only admitted provider base URL in v1; no proxies or custom processors.
pub const BASE_URL: &str = "https://api.openai.com/v1";
/// The only admitted transcription endpoint.
pub const ENDPOINT: &str = "https://api.openai.com/v1/audio/transcriptions";
/// Published list price, integer micro-USD per audio second ($0.0045/min).
const PRICE_MICROS_PER_SECOND: u64 = 75;
const POLICY_URL: &str = "https://developers.openai.com/api/docs/guides/your-data";
const DOCS_URL: &str = "https://developers.openai.com/api/docs/guides/speech-to-text";
/// File mode is the only mode this adapter advertises.
const MODE: &str = "file";
/// Product ASR languages. Extend this data table only with documented API support.
pub const LANGUAGES: [&str; 3] = ["vi", "en", "zh"];
/// Settings identity for composition-owned cache keys.
pub const SETTINGS_FINGERPRINT: &str =
    "openai-transport-v1;model=gpt-transcribe;format=json;wav=pcm16;languages=vi,en,zh";

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
    pub policy_url: NfcText,
    pub docs_url: NfcText,
}

/// Consent-gated OpenAI file transcription over an injected blocking transport.
pub struct OpenAiTranscriber {
    config: ProviderConfig,
    key: ApiKey,
    transport: Arc<dyn HttpTransport>,
    languages: Vec<LanguageTag>,
}
impl OpenAiTranscriber {
    /// Admits only the `gpt-transcribe` model, the `openai` backend and a remote
    /// versioned HTTPS base URL. The key is resolved by composition, never here.
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
            Location::Local { .. } => return Err(Error::Invalid("OpenAI requires remote config")),
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

    /// Canonical settings identity for the engine cache key.
    pub fn settings_fingerprint() -> &'static str {
        SETTINGS_FINGERPRINT
    }

    /// Provider identity, policy link, payload and cost for upload approval.
    pub fn disclosure(&self, scope: &Scope) -> Result<UploadDisclosure, Error> {
        self.validate_scope(scope)?;
        let audio_micros = duration_micros(scope)?;
        let upload_bytes = encoded_bytes(scope)?;
        Ok(UploadDisclosure {
            operation: Operation::Transcribe,
            provider: NfcText::from("OpenAI"),
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
            policy_url: NfcText::from(POLICY_URL),
            docs_url: NfcText::from(DOCS_URL),
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
impl fmt::Debug for OpenAiTranscriber {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenAiTranscriber")
            .field("config", &self.config)
            .field("key", &self.key)
            .field("transport", &"[transport]")
            .field("languages", &self.languages)
            .finish()
    }
}
impl Transcriber for OpenAiTranscriber {
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
            timing: BTreeSet::new(),
            diarization: Support::Unsupported,
            live_audio_input: Support::Unsupported,
            file_result_stream: Support::Unsupported,
            max_upload_bytes: Some(MAX_UPLOAD_BYTES),
            // The documented limit is a 25 MB file, not a fixed duration.
            max_duration_micros: None,
            evidence: Evidence::Advertised,
            revision: NfcText::from("gpt-transcribe-file-2026-11-04"),
        }
    }

    fn estimate(&self, scope: &Scope) -> CostEstimate {
        let Some(seconds) = rounded_up_seconds(scope) else {
            return CostEstimate::Unknown;
        };
        let Some(micros) = seconds.checked_mul(PRICE_MICROS_PER_SECOND) else {
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
        let segments = parse_transcription(&response, audio.range)?;
        checkpoint(hooks, Stage::Transcribe)?;
        Ok(segments)
    }
}

/// A network request exists only after consent, capability and audio checks pass.
struct AuthorizedUpload<'a> {
    scope: &'a Scope,
    key: &'a ApiKey,
}

impl OpenAiTranscriber {
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
        // Consent names the official endpoint; never
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

fn rounded_up_seconds(scope: &Scope) -> Option<u64> {
    let samples = u128::from(scope.source_samples.len());
    let rate = u128::from(scope.sample_rate);
    if rate == 0 {
        return None;
    }
    let seconds = samples.checked_add(rate - 1)? / rate;
    u64::try_from(seconds).ok()
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
    form.field("model", MODEL);
    form.field("response_format", "json");
    form.field("languages[]", upload.scope.source_language.as_str());
    form.file("audio.wav", "audio/wav", &audio);
    let (content_type, body) = form.finish();
    Ok(HttpRequest {
        method: "POST",
        url: ENDPOINT.into(),
        headers: vec![
            (
                "Authorization".into(),
                format!("Bearer {}", upload.key.expose()),
            ),
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
    languages: Vec<DetectedLanguage>,
}
#[derive(Deserialize)]
struct DetectedLanguage {
    code: String,
}

fn parse_transcription(
    response: &HttpResponse,
    window: SampleRange,
) -> Result<Vec<RecognitionSegment>, Error> {
    if response.body.len() > MAX_RESPONSE_BYTES {
        return Err(Error::Payload);
    }
    if !(200..=299).contains(&response.status) {
        return Err(status_error(response));
    }
    let parsed: Transcription =
        serde_json::from_slice(&response.body).map_err(|_| Error::Schema)?;
    for language in &parsed.languages {
        if language.code.trim().is_empty() || language.code.len() > 64 {
            return Err(Error::Schema);
        }
    }
    if parsed.text.is_empty() {
        return Ok(Vec::new());
    }
    Ok(vec![RecognitionSegment {
        id: NfcText::from(format!("openai-{}-{}-0", window.start(), window.end())),
        text: NfcText::from(parsed.text),
        range: None,
        timing: Vec::new(),
        raw_tokens: Vec::new(),
    }])
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: Option<ErrorDetail>,
}
#[derive(Deserialize)]
struct ErrorDetail {
    code: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
}

pub(crate) fn status_error(response: &HttpResponse) -> Error {
    match response.status {
        401 | 403 => Error::Auth,
        413 => Error::Payload,
        429 if quota_error(&response.body) => Error::Quota,
        429 => Error::RateLimit {
            retry_after_seconds: retry_after(&response.headers),
        },
        500..=599 => Error::Server,
        300..=399 => Error::Invalid("provider redirect"),
        400 | 404 | 409 | 422 => Error::Invalid("provider rejected request"),
        _ => Error::Server,
    }
}

fn quota_error(body: &[u8]) -> bool {
    let Ok(envelope) = serde_json::from_slice::<ErrorEnvelope>(body) else {
        return false;
    };
    envelope.error.is_some_and(|detail| {
        [detail.code, detail.kind]
            .into_iter()
            .flatten()
            .any(|value| {
                let value = value.to_ascii_lowercase();
                value.contains("quota") || value.contains("billing")
            })
    })
}
