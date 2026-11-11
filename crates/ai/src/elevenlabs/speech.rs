//! ElevenLabs text-to-speech adapter: consent-gated HTTPS, per-model language
//! capabilities, exact per-character cost and validated 24 kHz PCM.
use crate::remote::{self, UploadDisclosure};
use crate::speech::{
    MAX_SPEECH_TEXT_BYTES, SpeechAudio, SpeechCapabilities, SpeechFormat, SpeechLanguage,
    SpeechOutput, SpeechPolicy, SpeechRequest, SpeechSynthesizer, SpeechUsage, SpeechVoice,
    SynthesisConsent, SynthesisEstimate, prepare_synthesis, validate_speech_audio,
};
use crate::transport::{ApiKey, HttpRequest, HttpResponse, HttpTransport, retry_after};
use crate::{Error, ExecutionHooks, MAX_PCM_BYTES, Money, Operation, ProviderConfig, Support};
use captions::{LanguageTag, NfcText, NormalizedText};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

const BACKEND: &str = "elevenlabs";
const BASE_URL: &str = "https://api.elevenlabs.io/v1";
pub const OUTPUT_FORMAT: &str = "pcm_24000";
pub const SAMPLE_RATE: u32 = 24_000;
/// Bound explicit voice allowlists; each ID is a URL path segment.
pub const MAX_VOICES: usize = 1_000;
/// Continuity context sent to the provider; it is never synthesized.
pub const MAX_CONTINUITY_CHARACTERS: usize = 1_000;
/// Serialized JSON cap for one chunk, including escaping and context.
pub const MAX_REQUEST_BODY_BYTES: usize = 512 * 1024;
const NORMALIZATION: &str = "auto";
const POLICY_URL: &str = "https://elevenlabs.io/privacy-policy";
const TTS_DOC_URL: &str = "https://elevenlabs.io/docs/api-reference/text-to-speech/convert";
const MODELS_DOC_URL: &str = "https://elevenlabs.io/docs/overview/models";
const PRICING_URL: &str = "https://elevenlabs.io/pricing/api";
const ZRM_DOC_URL: &str = "https://elevenlabs.io/docs/eleven-api/resources/zero-retention-mode";

/// Default logging keeps provider history. Zero retention is requested per request
/// with `enable_logging=false`; it is Enterprise-only and the adapter cannot verify
/// the account's state, so this mode never claims that zero retention took effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoggingMode {
    Default,
    ZeroRetentionRequested,
}
impl LoggingMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::ZeroRetentionRequested => "zero-retention-requested",
        }
    }
}

/// Documented `voice_settings.speed` range, applied before duration fitting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpeechSpeed(f64);
impl SpeechSpeed {
    pub fn new(value: f64) -> Result<Self, Error> {
        if !value.is_finite() || !(0.7..=1.2).contains(&value) {
            return Err(Error::Invalid("speech speed"));
        }
        Ok(Self(value))
    }
    pub fn get(self) -> f64 {
        self.0
    }
    /// Exact wire representation, so two accepted speeds can never share a
    /// provider/capability identity. `serde_json` uses a round-trip float format;
    /// the fallback is unreachable because construction rejects non-finite values.
    fn fingerprint(self) -> String {
        serde_json::to_string(&self.0).unwrap_or_else(|_| self.0.to_string())
    }
}

/// Adapter-owned settings that must also appear in the provider configuration's
/// settings fingerprint, so consent cannot silently change them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElevenLabsSpeechSettings {
    pub speed: Option<SpeechSpeed>,
    pub logging: LoggingMode,
}
impl Default for ElevenLabsSpeechSettings {
    fn default() -> Self {
        Self {
            speed: None,
            logging: LoggingMode::Default,
        }
    }
}
impl ElevenLabsSpeechSettings {
    /// Exact value the provider config must carry, including normalization.
    pub fn settings_fingerprint(&self) -> String {
        let speed = self
            .speed
            .map(SpeechSpeed::fingerprint)
            .unwrap_or_else(|| "none".into());
        format!(
            "elevenlabs-speech-v1;format={OUTPUT_FORMAT};normalization={NORMALIZATION};speed={speed};logging={}",
            self.logging.as_str()
        )
    }
    fn speed_value(&self) -> Option<f64> {
        self.speed.map(SpeechSpeed::get)
    }
}

/// One ElevenLabs provider with per-model, per-language capabilities. Voices are
/// an explicit premade/stock allowlist supplied by composition; the adapter never
/// lists, selects or falls back to another voice or model.
pub struct ElevenLabsSpeech {
    config: ProviderConfig,
    key: ApiKey,
    transport: Arc<dyn HttpTransport>,
    settings: ElevenLabsSpeechSettings,
    model: SpeechProfile,
    voices: Vec<SpeechVoice>,
    languages: Vec<SpeechLanguage>,
}
impl ElevenLabsSpeech {
    /// The one admitted configuration for this model id and adapter settings.
    pub fn provider_config(
        model: &str,
        settings: &ElevenLabsSpeechSettings,
    ) -> Result<ProviderConfig, Error> {
        remote::provider_config(
            model,
            &settings.settings_fingerprint(),
            BACKEND,
            BASE_URL,
            "elevenlabs",
            "ELEVENLABS_API_KEY",
        )
    }

    /// Admits the two documented TTS models, backend `elevenlabs`, the official
    /// base URL and the exact settings fingerprint. Voice IDs must be premade or
    /// stock and URL-safe; cloned/professional-clone IDs are composition-owned and
    /// must not be supplied.
    pub fn new(
        config: ProviderConfig,
        key: ApiKey,
        transport: Arc<dyn HttpTransport>,
        settings: ElevenLabsSpeechSettings,
        voices: Vec<SpeechVoice>,
    ) -> Result<Self, Error> {
        let model = SpeechProfile::for_config(&config);
        remote::admit(
            &config,
            Some(&settings.settings_fingerprint()),
            BACKEND,
            BASE_URL,
            Some("ElevenLabs requires remote config"),
        )?;
        let voices = validate_voices(voices)?;
        let languages = model.languages()?;
        Ok(Self {
            config,
            key,
            transport,
            settings,
            model,
            voices,
            languages,
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct SpeechProfile {
    max_characters: u64,
    micros_per_character: Option<u64>,
    language_code: bool,
    vietnamese: bool,
}
impl SpeechProfile {
    fn for_config(config: &ProviderConfig) -> Self {
        if let Some(crate::defaults::Price::Character {
            micros,
            max_characters,
            language_code,
            vietnamese,
        }) = crate::defaults::price(config, Operation::Synthesize)
        {
            Self {
                max_characters,
                micros_per_character: Some(micros),
                language_code,
                vietnamese,
            }
        } else {
            Self {
                max_characters: 10_000,
                micros_per_character: None,
                language_code: false,
                vietnamese: true,
            }
        }
    }
    fn max_characters(self) -> u64 {
        self.max_characters
    }
    fn sends_language_code(self) -> bool {
        self.language_code
    }
    fn languages(self) -> Result<Vec<SpeechLanguage>, Error> {
        let mut languages = vec![
            language("en", "Latn")?,
            language("zh", "Hans")?,
            language("zh", "Hant")?,
            language("zh-Hans", "Hans")?,
            language("zh-Hant", "Hant")?,
        ];
        if self.vietnamese {
            languages.push(language("vi", "Latn")?);
        }
        Ok(languages)
    }
}

fn language(tag: &str, script: &str) -> Result<SpeechLanguage, Error> {
    Ok(SpeechLanguage {
        language: LanguageTag::parse(tag)?,
        script: NfcText::from(script),
    })
}

impl ElevenLabsSpeech {
    pub fn voices(&self) -> &[SpeechVoice] {
        &self.voices
    }

    fn policy(&self) -> SpeechPolicy {
        let retention = match self.settings.logging {
            LoggingMode::Default => {
                "ElevenLabs retains request history by default; change history retention in the provider dashboard."
            }
            LoggingMode::ZeroRetentionRequested => {
                "This request sets enable_logging=false. ElevenLabs applies zero retention only to confirmed Enterprise accounts and this adapter does not verify the account."
            }
        };
        SpeechPolicy {
            policy_url: NfcText::from(POLICY_URL),
            retention: NfcText::from(retention),
            training: NfcText::from(
                "ElevenLabs may use request data to improve its models unless the account opts out in the provider dashboard.",
            ),
            voice_disclosure: NfcText::from(
                "AI-generated speech from a premade stock voice; no voice cloning is requested.",
            ),
        }
    }

    fn fingerprint(&self, request: &SpeechRequest) -> NfcText {
        let speed = self
            .settings
            .speed
            .map(SpeechSpeed::fingerprint)
            .unwrap_or_else(|| "none".into());
        let language = format!(
            "{}/{}",
            request.language().language.as_str(),
            request.language().script.as_str()
        );
        NfcText::from(format!(
            "elevenlabs-speech-v1;model={};voice={};language={language};format={OUTPUT_FORMAT};normalization={NORMALIZATION};speed={speed};logging={};continuity={MAX_CONTINUITY_CHARACTERS};split=chunk-v1",
            self.config.model().as_str(),
            request.voice().as_str(),
            self.settings.logging.as_str(),
        ))
    }

    fn estimate_inner(&self, request: &SpeechRequest) -> SynthesisEstimate {
        if request.provider() != &self.config
            || request.output()
                != (SpeechOutput {
                    format: SpeechFormat::Pcm16,
                    sample_rate: SAMPLE_RATE,
                })
        {
            return SynthesisEstimate::Unknown;
        }
        let (characters, exact) = billable_count(request.text().as_str());
        let Some(micros) = self
            .model
            .micros_per_character
            .and_then(|rate| characters.checked_mul(rate))
        else {
            return SynthesisEstimate::Unknown;
        };
        let Ok(cost) = Money::new("USD", micros) else {
            return SynthesisEstimate::Unknown;
        };
        if exact {
            SynthesisEstimate::Exact(cost)
        } else {
            // Non-BMP text differs between Unicode scalars and UTF-16 units. Use the
            // larger documented count so the pre-call quote bounds provider spending.
            SynthesisEstimate::UpperBound(cost)
        }
    }
}
impl fmt::Debug for ElevenLabsSpeech {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ElevenLabsSpeech")
            .field("config", &self.config)
            .field("key", &self.key)
            .field("transport", &"[transport]")
            .field("settings", &self.settings)
            .field("model", &self.model)
            .field("voices", &self.voices.len())
            .finish()
    }
}

impl SpeechSynthesizer for ElevenLabsSpeech {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    fn capabilities(&self, request: &SpeechRequest) -> SpeechCapabilities {
        SpeechCapabilities {
            provider: self.config.clone(),
            fingerprint: self.fingerprint(request),
            languages: self.languages.clone(),
            voices: self.voices.clone(),
            outputs: vec![SpeechOutput {
                format: SpeechFormat::Pcm16,
                sample_rate: SAMPLE_RATE,
            }],
            // The contract already bounds request bytes; this is a safe grapheme
            // upper bound, not the per-request provider model limit that we split at.
            max_characters: Some(MAX_SPEECH_TEXT_BYTES as u64),
            streaming: Support::Unsupported,
            max_concurrency: None,
            requests_per_minute: None,
            cost_model: NfcText::from(if self.model.micros_per_character.is_some() {
                "documented per-character price"
            } else {
                "price unknown"
            }),
            policy: self.policy(),
        }
    }

    fn estimate(&self, request: &SpeechRequest) -> SynthesisEstimate {
        self.estimate_inner(request)
    }

    fn disclosure(&self, request: &SpeechRequest) -> Result<Option<UploadDisclosure>, Error> {
        self.capabilities(request).check(request)?;
        Ok(Some(remote::disclosure(remote::Disclosure {
            operation: Operation::Synthesize,
            provider: "ElevenLabs",
            model: self.config.model(),
            languages: NfcText::from(format!(
                "{}/{}",
                request.language().language.as_str(),
                request.language().script.as_str()
            )),
            audio_micros: request.max_duration_micros(),
            upload_bytes: u64::try_from(request.text().as_str().len())
                .map_err(|_| Error::Payload)?,
            estimate: remote::cost_ceiling(self.estimate(request)),
            links: &[
                ("policy", POLICY_URL),
                ("docs", TTS_DOC_URL),
                ("models", MODELS_DOC_URL),
                ("pricing", PRICING_URL),
                ("zero_retention", ZRM_DOC_URL),
            ],
        })))
    }

    fn synthesize(
        &self,
        request: &SpeechRequest,
        consent: &SynthesisConsent,
        hooks: &dyn ExecutionHooks,
    ) -> Result<SpeechAudio, Error> {
        prepare_synthesis(self, request, consent)?;
        let capabilities = self.capabilities(request);
        let estimate = self.estimate_inner(request);
        checked(hooks)?;
        let chunks = split_text(
            request.text().as_str(),
            self.model.max_characters() as usize,
        )?;
        let cap = response_cap(request)?;
        let max_samples = max_samples(request)?;
        let mut samples = Vec::new();
        let mut characters = 0u64;
        let mut provider_cost = Some(0u64);
        for (index, chunk) in chunks.iter().enumerate() {
            checked(hooks)?;
            let previous = match index.checked_sub(1).and_then(|i| chunks.get(i)) {
                Some(text) => nonempty(suffix_scalars(text, MAX_CONTINUITY_CHARACTERS)?),
                None => None,
            };
            let next = match chunks.get(index + 1) {
                Some(text) => nonempty(prefix_scalars(text, MAX_CONTINUITY_CHARACTERS)?),
                None => None,
            };
            let body = build_body(
                chunk,
                self.model,
                request,
                previous.as_deref(),
                next.as_deref(),
                self.settings,
            )?;
            let outbound = build_http(request, &self.key, self.settings, body)?;
            checked(hooks)?;
            let response = self.transport.send_bounded(outbound, cap)?;
            remote::check_model_response(&response, &self.config)?;
            checked(hooks)?;
            let remaining = max_samples.saturating_sub(samples.len());
            if (200..=299).contains(&response.status) && response.body.len() / 2 > remaining {
                return Err(Error::OutputRejected);
            }
            let decoded = parse_chunk(&response)?;
            characters = characters
                .checked_add(billable_count(chunk).0)
                .ok_or(Error::Cost)?;
            provider_cost = match (provider_cost, decoded.provider_cost) {
                (Some(total), Some(cost)) => total.checked_add(cost),
                _ => None,
            };
            let total = samples
                .len()
                .checked_add(decoded.samples.len())
                .ok_or(Error::OutputRejected)?;
            if total.saturating_mul(4) as u64 > MAX_PCM_BYTES
                || (total as u128).checked_mul(1_000_000).is_none_or(|micros| {
                    micros > u128::from(request.max_duration_micros()) * u128::from(SAMPLE_RATE)
                })
            {
                return Err(Error::OutputRejected);
            }
            samples.extend(decoded.samples);
            checked(hooks)?;
        }
        let duration_micros = duration_micros(samples.len())?;
        // `character-cost` is the provider's billing-unit count (Flash reports half
        // a unit per input character), so it is recorded, not multiplied by the
        // per-character list price. Actual cost follows the documented input-text
        // character rate; otherwise Flash would be underreported by half.
        let actual_cost = self
            .model
            .micros_per_character
            .map(|rate| characters.max(1).checked_mul(rate).ok_or(Error::Cost))
            .transpose()?
            .map(|micros| Money::new("USD", micros))
            .transpose()?;
        let audio = SpeechAudio {
            samples,
            sample_rate: SAMPLE_RATE,
            duration_micros,
            fingerprint: capabilities.fingerprint.clone(),
            usage: SpeechUsage {
                billable_characters: provider_cost,
                input_tokens: None,
                output_tokens: None,
                actual_cost,
            },
            transcript: None,
        };
        validate_speech_audio(&audio, request, &capabilities, &estimate)?;
        checked(hooks)?;
        Ok(audio)
    }
}

fn validate_voices(voices: Vec<SpeechVoice>) -> Result<Vec<SpeechVoice>, Error> {
    if voices.is_empty() || voices.len() > MAX_VOICES {
        return Err(Error::Invalid("ElevenLabs stock voices"));
    }
    let mut seen = BTreeSet::new();
    for voice in &voices {
        let id = voice.id.as_str();
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
            || !seen.insert(id)
        {
            return Err(Error::Invalid("ElevenLabs stock voice ID"));
        }
    }
    Ok(voices)
}

fn billable_count(text: &str) -> (u64, bool) {
    // Providers bill characters, not extended graphemes. ElevenLabs does not
    // publish whether non-BMP text is counted as scalars or UTF-16 units, so the
    // quote uses the larger count. BMP-only VI/EN/ZH text is exact.
    let scalars = text.chars().count();
    let utf16 = text.encode_utf16().count();
    let count = scalars.max(utf16).max(1);
    (u64::try_from(count).unwrap_or(u64::MAX), scalars == utf16)
}

fn duration_micros(samples: usize) -> Result<u64, Error> {
    let micros = (samples as u128)
        .checked_mul(1_000_000)
        .ok_or(Error::OutputRejected)?
        .div_ceil(u128::from(SAMPLE_RATE));
    u64::try_from(micros).map_err(|_| Error::OutputRejected)
}

fn max_samples(request: &SpeechRequest) -> Result<usize, Error> {
    let by_duration = u128::from(request.max_duration_micros())
        .checked_mul(u128::from(SAMPLE_RATE))
        .ok_or(Error::Payload)?
        / 1_000_000;
    // The shared f32 PCM ceiling is the second bound; PCM16 bytes are half of it.
    let samples = by_duration.min(u128::from(MAX_PCM_BYTES) / 4);
    usize::try_from(samples).map_err(|_| Error::Payload)
}

fn response_cap(request: &SpeechRequest) -> Result<usize, Error> {
    // A raw PCM16 body cannot be larger than the samples the approved duration and
    // the decoded f32 ceiling admit.
    let bytes = max_samples(request)?
        .checked_mul(2)
        .ok_or(Error::Payload)?
        .max(2);
    Ok(bytes)
}

fn checked(hooks: &dyn ExecutionHooks) -> Result<(), Error> {
    if hooks.cancelled() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

#[derive(Serialize)]
struct Body<'a> {
    text: &'a str,
    model_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    language_code: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    voice_settings: Option<VoiceSettings>,
    apply_text_normalization: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_text: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_text: Option<&'a str>,
}
#[derive(Serialize)]
struct VoiceSettings {
    speed: f64,
}

fn build_body(
    text: &str,
    model: SpeechProfile,
    request: &SpeechRequest,
    previous: Option<&str>,
    next: Option<&str>,
    settings: ElevenLabsSpeechSettings,
) -> Result<Vec<u8>, Error> {
    let language_code = if model.sends_language_code() {
        Some(request.language().language.primary())
    } else {
        None
    };
    let voice_settings = settings.speed_value().map(|speed| VoiceSettings { speed });
    let body = Body {
        text,
        model_id: request.provider().model().as_str(),
        language_code,
        voice_settings,
        apply_text_normalization: NORMALIZATION,
        previous_text: previous,
        next_text: next,
    };
    let body = serde_json::to_vec(&body).map_err(|_| Error::Payload)?;
    if body.len() > MAX_REQUEST_BODY_BYTES {
        return Err(Error::Payload);
    }
    Ok(body)
}

fn nonempty(value: String) -> Option<String> {
    if value.is_empty() { None } else { Some(value) }
}

fn build_http(
    request: &SpeechRequest,
    key: &ApiKey,
    settings: ElevenLabsSpeechSettings,
    body: Vec<u8>,
) -> Result<HttpRequest, Error> {
    let mut url = format!(
        "{BASE_URL}/text-to-speech/{}?output_format={OUTPUT_FORMAT}",
        request.voice().as_str()
    );
    if settings.logging == LoggingMode::ZeroRetentionRequested {
        url.push_str("&enable_logging=false");
    }
    Ok(HttpRequest {
        method: "POST",
        url,
        headers: vec![
            ("xi-api-key".into(), key.expose().to_owned()),
            ("Content-Type".into(), "application/json".into()),
        ],
        body,
    })
}

struct ChunkAudio {
    samples: Vec<f32>,
    provider_cost: Option<u64>,
}

fn parse_chunk(response: &HttpResponse) -> Result<ChunkAudio, Error> {
    // This endpoint has no completion-status field: a single 200 response carries
    // the whole audio body. Other 2xx statuses (204, 206, ...) are incomplete or
    // partial and are rejected rather than treated as success.
    if response.status != 200 {
        return if (200..=299).contains(&response.status) {
            Err(Error::OutputRejected)
        } else {
            Err(status_error(response))
        };
    }
    if let Some(declared) = declared_length(&response.headers)? {
        let actual = u64::try_from(response.body.len()).map_err(|_| Error::OutputRejected)?;
        if declared != actual {
            return Err(Error::OutputRejected);
        }
    }
    if response.body.is_empty() || !response.body.len().is_multiple_of(2) {
        return Err(Error::OutputRejected);
    }
    let mut samples = Vec::with_capacity(response.body.len() / 2);
    for pair in response.body.as_chunks::<2>().0 {
        samples.push(f32::from(i16::from_le_bytes(*pair)) / 32768.0);
    }
    let provider_cost = header_value(&response.headers, "character-cost")
        .and_then(|value| value.trim().parse().ok());
    Ok(ChunkAudio {
        samples,
        provider_cost,
    })
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header, _)| header.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

/// Raw PCM has no container length of its own; when the provider declares an HTTP
/// body length, the received bytes must match it so a truncated or extended body
/// cannot pass as complete audio. Chunked responses legitimately omit it.
fn declared_length(headers: &[(String, String)]) -> Result<Option<u64>, Error> {
    match header_value(headers, "content-length") {
        Some(value) => value
            .trim()
            .parse()
            .map(Some)
            .map_err(|_| Error::OutputRejected),
        None => Ok(None),
    }
}

fn status_error(response: &HttpResponse) -> Error {
    // ElevenLabs documents `quota_exceeded` under both HTTP 400 and 401. Classify
    // bounded provider codes before the generic status mapping so exhausted credits
    // stay distinct from bad input and invalid credentials. Codes never enter the
    // public error or logs.
    remote::status_error(response, |status, fields| {
        if (400..=429).contains(&status) {
            if remote::quota(fields) {
                return Some(Error::Quota);
            }
            if remote::matches(
                fields,
                &["invalid_api_key", "authentication", "unauthorized"],
            ) {
                return Some(Error::Auth);
            }
            if remote::matches(fields, &["content_policy", "moderation", "safety"]) {
                return Some(Error::ContentPolicy);
            }
            if remote::matches(fields, &["rate_limit", "too_many_requests"]) {
                return Some(Error::RateLimit {
                    retry_after_seconds: retry_after(&response.headers),
                });
            }
        }
        match status {
            400 | 422 => Some(Error::Payload),
            402 => Some(Error::Quota),
            404 => Some(Error::UnsupportedVoice),
            408 => Some(Error::Timeout),
            409 => Some(Error::Invalid("provider rejected request")),
            _ => None,
        }
    })
}

/// Split once at a cue/newline, sentence or whitespace boundary at or before the
/// model limit; otherwise at the last whole grapheme that fits. Text is preserved
/// exactly across the concatenated chunks.
fn split_text(text: &str, limit: usize) -> Result<Vec<String>, Error> {
    if limit == 0 {
        return Err(Error::Payload);
    }
    let normalized = NormalizedText::new(text).map_err(|_| Error::Payload)?;
    let source = normalized.text().as_str();
    if source != text {
        return Err(Error::Payload);
    }
    let spans = normalized.spans();
    if spans.is_empty() {
        return Err(Error::Payload);
    }
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < spans.len() {
        let mut scalars = 0usize;
        let mut end = start;
        while end < spans.len() {
            let span = spans.get(end).ok_or(Error::Payload)?;
            let grapheme = source.get(span.normalized.clone()).ok_or(Error::Payload)?;
            let count = grapheme.chars().count();
            if scalars.saturating_add(count) > limit {
                break;
            }
            scalars = scalars.saturating_add(count);
            end = end.saturating_add(1);
        }
        if end == start {
            return Err(Error::Payload);
        }
        let break_at = if end == spans.len() {
            end
        } else {
            choose_break(&normalized, start, end)?
        };
        let start_byte = spans.get(start).ok_or(Error::Payload)?.normalized.start;
        let end_byte = if break_at == spans.len() {
            source.len()
        } else {
            spans.get(break_at).ok_or(Error::Payload)?.normalized.start
        };
        let chunk = source.get(start_byte..end_byte).ok_or(Error::Payload)?;
        if chunk.is_empty() {
            return Err(Error::Payload);
        }
        chunks.push(chunk.to_owned());
        start = break_at;
    }
    if chunks.is_empty() {
        return Err(Error::Payload);
    }
    Ok(chunks)
}

fn choose_break(normalized: &NormalizedText, start: usize, end: usize) -> Result<usize, Error> {
    if end <= start.saturating_add(1) {
        return Ok(end);
    }
    let minimum = start.saturating_add((end - start) / 2);
    let mut newline = None;
    let mut sentence = None;
    let mut whitespace = None;
    for index in start..end {
        let span = normalized.spans().get(index).ok_or(Error::Payload)?;
        let grapheme = normalized
            .text()
            .as_str()
            .get(span.normalized.clone())
            .ok_or(Error::Payload)?;
        let boundary = index.saturating_add(1);
        if boundary > minimum && (grapheme.contains('\n') || grapheme.contains('\r')) {
            newline = Some(boundary);
        } else if boundary > minimum && is_sentence_end(grapheme) {
            sentence = Some(boundary);
        } else if boundary > minimum && grapheme.chars().all(char::is_whitespace) {
            whitespace = Some(boundary);
        }
    }
    let mut break_at = newline.or(sentence).or(whitespace).unwrap_or(end);
    // Keep trailing whitespace with the preceding sentence/space so the next chunk
    // starts on visible text where possible.
    while break_at < end {
        let span = normalized.spans().get(break_at).ok_or(Error::Payload)?;
        let grapheme = normalized
            .text()
            .as_str()
            .get(span.normalized.clone())
            .ok_or(Error::Payload)?;
        if grapheme.chars().all(char::is_whitespace) {
            break_at = break_at.saturating_add(1);
        } else {
            break;
        }
    }
    Ok(break_at)
}

fn is_sentence_end(grapheme: &str) -> bool {
    grapheme
        .chars()
        .any(|character| matches!(character, '.' | '!' | '?' | '…' | '。' | '！' | '？'))
}

fn prefix_scalars(text: &str, maximum: usize) -> Result<String, Error> {
    let normalized = NormalizedText::new(text).map_err(|_| Error::Payload)?;
    let source = normalized.text().as_str();
    let mut total = 0usize;
    let mut end = 0usize;
    while let Some(span) = normalized.spans().get(end) {
        let grapheme = source.get(span.normalized.clone()).ok_or(Error::Payload)?;
        let count = grapheme.chars().count();
        if total.saturating_add(count) > maximum {
            break;
        }
        total = total.saturating_add(count);
        end = end.saturating_add(1);
    }
    let byte = normalized
        .spans()
        .get(end)
        .map(|span| span.normalized.start)
        .unwrap_or(source.len());
    Ok(source.get(..byte).ok_or(Error::Payload)?.to_owned())
}

fn suffix_scalars(text: &str, maximum: usize) -> Result<String, Error> {
    let normalized = NormalizedText::new(text).map_err(|_| Error::Payload)?;
    let source = normalized.text().as_str();
    let mut total = 0usize;
    let mut start = normalized.spans().len();
    while start > 0 {
        let index = start.saturating_sub(1);
        let span = normalized.spans().get(index).ok_or(Error::Payload)?;
        let grapheme = source.get(span.normalized.clone()).ok_or(Error::Payload)?;
        let count = grapheme.chars().count();
        if total.saturating_add(count) > maximum {
            break;
        }
        total = total.saturating_add(count);
        start = index;
    }
    let byte = normalized
        .spans()
        .get(start)
        .map(|span| span.normalized.start)
        .unwrap_or(source.len());
    Ok(source.get(byte..).ok_or(Error::Payload)?.to_owned())
}
