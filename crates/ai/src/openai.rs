//! OpenAI file transcription: consent-gated HTTPS transport, typed failures and
//! text-only recognition. Timing requires an explicitly selected aligner.
mod translation;
use crate::contracts::checkpoint;
use crate::{
    AudioWindow, Capabilities, CostEstimate, Error, Evidence, ExecutionHooks, Location, Money,
    Operation, ProviderConfig, RecognitionSegment, Request, SampleRange, Scope, Stage, Support,
    Transcriber,
};
use captions::{LanguageTag, NfcText};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fmt;
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
pub use translation::*;

/// The only production model admitted here. Deprecated transcription models are refused.
pub const MODEL: &str = "gpt-transcribe";
/// Provider backend name expected in the tagged remote configuration.
pub const BACKEND: &str = "openai";
/// Current documented file limit for `/v1/audio/transcriptions` (25 MB decimal).
pub const MAX_UPLOAD_BYTES: u64 = 25_000_000;
/// Successful transcripts and error envelopes are small; cap hostile responses.
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
/// The only admitted provider base URL in v1; no proxies or custom processors.
pub const BASE_URL: &str = "https://api.openai.com/v1";
/// The only admitted transcription endpoint.
pub const ENDPOINT: &str = "https://api.openai.com/v1/audio/transcriptions";
/// Published list price, integer micro-USD per audio second ($0.0045/min).
const PRICE_MICROS_PER_SECOND: u64 = 75;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const POLICY_URL: &str = "https://developers.openai.com/api/docs/guides/your-data";
const DOCS_URL: &str = "https://developers.openai.com/api/docs/guides/speech-to-text";
const TRAINING_DISCLOSURE: &str = "OpenAI documents that data sent to /v1/audio/transcriptions is not used for training unless the account explicitly opts in (checked 2026-11-04).";
const RETENTION_DISCLOSURE: &str = "OpenAI's data-controls table lists no abuse-monitoring retention and no application-state retention for /v1/audio/transcriptions; Zero Data Retention eligibility requires account approval and policy exceptions apply (checked 2026-11-04).";
/// File mode is the only mode this adapter advertises.
const MODE: &str = "file";
/// Product ASR languages. Extend this data table only with documented API support.
const LANGUAGES: [&str; 3] = ["vi", "en", "zh"];
/// Settings identity for composition-owned cache keys.
pub const SETTINGS_FINGERPRINT: &str =
    "openai-transport-v1;model=gpt-transcribe;format=json;wav=pcm16;languages=vi,en,zh";

/// In-memory API key injected by the composition root. Never serialized or logged.
#[derive(Clone)]
pub struct ApiKey(String);
impl ApiKey {
    /// Validates a printable ASCII key of at most 4096 bytes; Debug always redacts it.
    pub fn new(value: impl Into<String>) -> Result<Self, Error> {
        let value = value.into();
        if value.is_empty() || value.trim() != value || !value.is_ascii() || value.len() > 4096 {
            return Err(Error::Invalid("API key"));
        }
        Ok(Self(value))
    }
    fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for ApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ApiKey([redacted])")
    }
}

/// Transport request; Debug never prints headers (authorization) or the body (audio).
pub struct HttpRequest {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl fmt::Debug for HttpRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &"[redacted]")
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

/// Provider response; Debug never prints the body (transcript or error text).
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl fmt::Debug for HttpResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("headers", &"[redacted]")
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

/// Blocking HTTPS seam. Implementations must bound connect/read/write time and
/// must not retry; the engine job runner owns the attempt budget and backoff.
pub trait HttpTransport: Send + Sync {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, Error>;
}

/// Production transport with connect/read/write/total timeouts and redirects disabled
/// so an authorization header can never follow a redirect to another host.
pub struct UreqTransport {
    agent: ureq::Agent,
}
impl UreqTransport {
    /// Uses a 120 s total/read/write timeout, a 10 s connect timeout and no redirects.
    pub fn new() -> Self {
        Self::agent(REQUEST_TIMEOUT)
    }
    /// Same policy with an explicit nonzero total/read/write timeout.
    pub fn with_timeout(timeout: Duration) -> Result<Self, Error> {
        if timeout.is_zero() {
            return Err(Error::Invalid("transport timeout"));
        }
        Ok(Self::agent(timeout))
    }
    fn agent(timeout: Duration) -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout_connect(timeout.min(Duration::from_secs(10)))
                .timeout_read(timeout)
                .timeout_write(timeout)
                .timeout(timeout)
                .redirects(0)
                .build(),
        }
    }
    fn read_response(response: ureq::Response) -> Result<HttpResponse, Error> {
        let status = response.status();
        let mut headers = Vec::new();
        for name in response.headers_names() {
            if let Some(value) = response.header(&name) {
                headers.push((name, value.to_owned()));
            }
        }
        let mut body = Vec::new();
        response
            .into_reader()
            .take(MAX_RESPONSE_BYTES as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|_| Error::Network)?;
        if body.len() > MAX_RESPONSE_BYTES {
            return Err(Error::Payload);
        }
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}
impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}
impl HttpTransport for UreqTransport {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
        let mut outbound = self.agent.post(&request.url);
        for (name, value) in &request.headers {
            outbound = outbound.set(name, value);
        }
        match outbound.send_bytes(&request.body) {
            Ok(response) | Err(ureq::Error::Status(_, response)) => Self::read_response(response),
            Err(ureq::Error::Transport(_)) => Err(Error::Network),
        }
    }
}

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

    /// Current operation-specific retention/training disclosure with source links.
    /// Translation and other operations never inherit these claims because this
    /// adapter advertises transcription only.
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
        // The OpenAI retention/training disclosure is endpoint-specific; never
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
    let bytes = scope
        .source_samples
        .len()
        .checked_mul(2)
        .and_then(|data| data.checked_add(44))
        .ok_or(Error::Payload)?;
    if bytes > MAX_UPLOAD_BYTES {
        return Err(Error::Payload);
    }
    Ok(bytes)
}

fn build_request(
    upload: &AuthorizedUpload<'_>,
    audio: AudioWindow<'_>,
) -> Result<HttpRequest, Error> {
    let wav = encode_wav(&audio)?;
    let boundary = boundary(&wav)?;
    let mut body = Vec::new();
    field(&mut body, &boundary, "model", MODEL);
    field(&mut body, &boundary, "response_format", "json");
    field(
        &mut body,
        &boundary,
        "languages[]",
        upload.scope.source_language.as_str(),
    );
    file(&mut body, &boundary, "audio.wav", &wav);
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Ok(HttpRequest {
        method: "POST",
        url: ENDPOINT.into(),
        headers: vec![
            (
                "Authorization".into(),
                format!("Bearer {}", upload.key.expose()),
            ),
            (
                "Content-Type".into(),
                format!("multipart/form-data; boundary={boundary}"),
            ),
            ("Accept".into(), "application/json".into()),
        ],
        body,
    })
}

fn content_disposition(name: &str) -> String {
    format!("Content-Disposition: form-data; name=\"{name}\"")
}

fn field(body: &mut Vec<u8>, boundary: &str, name: &str, value: &str) {
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(content_disposition(name).as_bytes());
    body.extend_from_slice(b"\r\n\r\n");
    body.extend_from_slice(value.as_bytes());
    body.extend_from_slice(b"\r\n");
}

fn file(body: &mut Vec<u8>, boundary: &str, filename: &str, bytes: &[u8]) {
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"")
            .as_bytes(),
    );
    body.extend_from_slice(b"\r\nContent-Type: audio/wav\r\n\r\n");
    body.extend_from_slice(bytes);
    body.extend_from_slice(b"\r\n");
}

fn boundary(payload: &[u8]) -> Result<String, Error> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    for _ in 0..8 {
        let candidate = format!(
            "vibecut-{:x}-{:x}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        if !payload
            .windows(candidate.len())
            .any(|window| window == candidate.as_bytes())
        {
            return Ok(candidate);
        }
    }
    Err(Error::Invalid("multipart boundary"))
}

/// Mono 16-bit PCM WAV. Input is already validated finite mono f32.
fn encode_wav(audio: &AudioWindow<'_>) -> Result<Vec<u8>, Error> {
    let data_bytes = audio.samples.len().checked_mul(2).ok_or(Error::Payload)?;
    let total = data_bytes.checked_add(44).ok_or(Error::Payload)?;
    if u64::try_from(total).map_err(|_| Error::Payload)? > MAX_UPLOAD_BYTES {
        return Err(Error::Payload);
    }
    let data_len = u32::try_from(data_bytes).map_err(|_| Error::Payload)?;
    let riff_len = data_len.checked_add(36).ok_or(Error::Payload)?;
    let byte_rate = audio.sample_rate.checked_mul(2).ok_or(Error::Payload)?;
    let mut wav = Vec::with_capacity(total);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_len.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&audio.sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in audio.samples {
        let value = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        wav.extend_from_slice(&value.to_le_bytes());
    }
    Ok(wav)
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
        serde_json::from_slice(&response.body).map_err(|_| Error::Invalid("provider response"))?;
    for language in &parsed.languages {
        if language.code.trim().is_empty() || language.code.len() > 64 {
            return Err(Error::Invalid("provider language"));
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

fn retry_after(headers: &[(String, String)]) -> Option<u32> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("retry-after"))
        .and_then(|(_, value)| value.trim().parse().ok())
}
