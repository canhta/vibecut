//! ElevenLabs text-to-speech adapter: consent-gated HTTPS, per-model language
//! capabilities, exact per-character cost and validated 24 kHz PCM.
use crate::speech::{
    MAX_SPEECH_TEXT_BYTES, SpeechAudio, SpeechCapabilities, SpeechFormat, SpeechLanguage,
    SpeechOutput, SpeechPolicy, SpeechRequest, SpeechSynthesizer, SpeechUsage, SpeechVoice,
    SynthesisConsent, SynthesisEstimate, prepare_synthesis, validate_speech_audio,
};
use crate::transport::{ApiKey, HttpRequest, HttpResponse, HttpTransport, retry_after};
use crate::{
    Error, ExecutionHooks, Location, MAX_PCM_BYTES, Money, Operation, ProviderConfig, Support,
};
use captions::{LanguageTag, NfcText, NormalizedText};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

pub const BACKEND: &str = "elevenlabs";
pub const BASE_URL: &str = "https://api.elevenlabs.io/v1";
pub const MULTILINGUAL_MODEL: &str = "eleven_multilingual_v2";
pub const FLASH_MODEL: &str = "eleven_flash_v2_5";
pub const OUTPUT_FORMAT: &str = "pcm_24000";
pub const SAMPLE_RATE: u32 = 24_000;
/// Source-documented model limits.
pub const MULTILINGUAL_MAX_CHARACTERS: u64 = 10_000;
pub const FLASH_MAX_CHARACTERS: u64 = 40_000;
/// List prices in integer micro-USD per input character.
pub const MULTILINGUAL_MICROS_PER_CHARACTER: u64 = 80;
pub const FLASH_MICROS_PER_CHARACTER: u64 = 40;
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
    fn description(self) -> &'static str {
        match self {
            Self::Default => "default logging; history retention applies",
            Self::ZeroRetentionRequested => {
                "zero retention requested with enable_logging=false; Enterprise-only and not verified by this adapter"
            }
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
    fn fingerprint(self) -> String {
        format!("{:.2}", self.0)
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

/// What composition must show before granting a per-invocation synthesis consent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeechDisclosure {
    pub operation: Operation,
    pub provider: NfcText,
    pub model: NfcText,
    pub voice: NfcText,
    pub language: SpeechLanguage,
    pub output: SpeechOutput,
    pub characters: u64,
    /// Conservative billable count (Unicode scalars or UTF-16 units), minimum one.
    pub billable_characters: u64,
    pub text_sha256: NfcText,
    pub max_duration_micros: u64,
    pub speed: Option<NfcText>,
    pub logging: NfcText,
    pub fingerprint: NfcText,
    pub estimate: SynthesisEstimate,
    pub policy: SpeechPolicy,
    pub docs_url: NfcText,
    pub models_url: NfcText,
    pub pricing_url: NfcText,
    pub zero_retention_url: NfcText,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Model {
    MultilingualV2,
    FlashV25,
}
impl Model {
    fn parse(id: &str) -> Option<Self> {
        match id {
            MULTILINGUAL_MODEL => Some(Self::MultilingualV2),
            FLASH_MODEL => Some(Self::FlashV25),
            _ => None,
        }
    }
    fn id(self) -> &'static str {
        match self {
            Self::MultilingualV2 => MULTILINGUAL_MODEL,
            Self::FlashV25 => FLASH_MODEL,
        }
    }
    fn max_characters(self) -> u64 {
        match self {
            Self::MultilingualV2 => MULTILINGUAL_MAX_CHARACTERS,
            Self::FlashV25 => FLASH_MAX_CHARACTERS,
        }
    }
    fn micros_per_character(self) -> u64 {
        match self {
            Self::MultilingualV2 => MULTILINGUAL_MICROS_PER_CHARACTER,
            Self::FlashV25 => FLASH_MICROS_PER_CHARACTER,
        }
    }
    fn cost_model(self) -> &'static str {
        match self {
            Self::MultilingualV2 => {
                "USD 0.00008 per input character (80 micro-USD), minimum 1 character per request"
            }
            Self::FlashV25 => {
                "USD 0.00004 per input character (40 micro-USD), minimum 1 character per request"
            }
        }
    }
    /// `language_code` is documented for Flash v2.5 but not for Multilingual v2.
    /// Sending it to a model that ignores it would make the fingerprint dishonest.
    fn sends_language_code(self) -> bool {
        matches!(self, Self::FlashV25)
    }
    fn languages(self) -> Result<Vec<SpeechLanguage>, Error> {
        let mut languages = vec![language("en", "Latn")?];
        languages.push(language("zh", "Hans")?);
        languages.push(language("zh", "Hant")?);
        // Accept the script-qualified tags callers already use for captions while
        // keeping the provider's ISO 639-1 code for the wire request.
        languages.push(language("zh-Hans", "Hans")?);
        languages.push(language("zh-Hant", "Hant")?);
        if matches!(self, Self::FlashV25) {
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

/// One ElevenLabs provider with per-model, per-language capabilities. Voices are
/// an explicit premade/stock allowlist supplied by composition; the adapter never
/// lists, selects or falls back to another voice or model.
pub struct ElevenLabsSpeech {
    config: ProviderConfig,
    key: ApiKey,
    transport: Arc<dyn HttpTransport>,
    settings: ElevenLabsSpeechSettings,
    model: Model,
    voices: Vec<SpeechVoice>,
    languages: Vec<SpeechLanguage>,
}
impl ElevenLabsSpeech {
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
        let model = Model::parse(config.model().as_str()).ok_or(Error::Unsupported)?;
        if config.settings_fingerprint().as_str() != settings.settings_fingerprint() {
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

    /// Provider identity, exact model/voice/language and policy for the consent sheet.
    pub fn disclosure(&self, request: &SpeechRequest) -> Result<SpeechDisclosure, Error> {
        let capabilities = self.capabilities(request);
        capabilities.check(request)?;
        let billable = billable_count(request.text().as_str()).0;
        Ok(SpeechDisclosure {
            operation: Operation::Synthesize,
            provider: NfcText::from("ElevenLabs"),
            model: self.config.model().clone(),
            voice: request.voice().clone(),
            language: request.language().clone(),
            output: request.output(),
            characters: request.characters(),
            billable_characters: billable,
            text_sha256: NfcText::from(hex(&request.text_sha256())),
            max_duration_micros: request.max_duration_micros(),
            speed: self
                .settings
                .speed
                .map(|speed| NfcText::from(speed.fingerprint())),
            logging: NfcText::from(self.settings.logging.description()),
            fingerprint: capabilities.fingerprint,
            estimate: self.estimate(request),
            policy: self.policy(),
            docs_url: NfcText::from(TTS_DOC_URL),
            models_url: NfcText::from(MODELS_DOC_URL),
            pricing_url: NfcText::from(PRICING_URL),
            zero_retention_url: NfcText::from(ZRM_DOC_URL),
        })
    }

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
        let Some(micros) = characters.checked_mul(self.model.micros_per_character()) else {
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
            cost_model: NfcText::from(self.model.cost_model()),
            policy: self.policy(),
        }
    }

    fn estimate(&self, request: &SpeechRequest) -> SynthesisEstimate {
        self.estimate_inner(request)
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
        let mut billable = 0u64;
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
            checked(hooks)?;
            let remaining = max_samples.saturating_sub(samples.len());
            if (200..=299).contains(&response.status) && response.body.len() / 2 > remaining {
                return Err(Error::OutputRejected);
            }
            let decoded = parse_chunk(&response, chunk)?;
            billable = billable.checked_add(decoded.billable).ok_or(Error::Cost)?;
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
        let Some(actual_micros) = billable
            .max(1)
            .checked_mul(self.model.micros_per_character())
        else {
            return Err(Error::Cost);
        };
        let audio = SpeechAudio {
            samples,
            sample_rate: SAMPLE_RATE,
            duration_micros,
            fingerprint: capabilities.fingerprint.clone(),
            usage: SpeechUsage {
                billable_characters: Some(billable.max(1)),
                input_tokens: None,
                output_tokens: None,
                actual_cost: Money::new("USD", actual_micros)?,
            },
            transcript: None,
        };
        validate_speech_audio(&audio, request, &capabilities, &estimate)?;
        checked(hooks)?;
        Ok(audio)
    }
}

fn official_base_url(base_url: &str) -> Result<(), Error> {
    if base_url.trim_end_matches('/') == BASE_URL {
        Ok(())
    } else {
        // Consent names the official endpoint; never apply it to a proxy,
        // lookalike host or unsupported version path.
        Err(Error::Unsupported)
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
    model: Model,
    request: &SpeechRequest,
    previous: Option<&str>,
    next: Option<&str>,
    settings: ElevenLabsSpeechSettings,
) -> Result<Vec<u8>, Error> {
    let language_code = if model.sends_language_code() {
        Some(base_language(request.language().language.as_str()))
    } else {
        None
    };
    let voice_settings = settings.speed_value().map(|speed| VoiceSettings { speed });
    let body = Body {
        text,
        model_id: model.id(),
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

fn base_language(tag: &str) -> &str {
    tag.split('-').next().unwrap_or(tag)
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
    billable: u64,
}

fn parse_chunk(response: &HttpResponse, text: &str) -> Result<ChunkAudio, Error> {
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
    let billable = header_value(&response.headers, "character-cost")
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or_else(|| billable_count(text).0);
    Ok(ChunkAudio { samples, billable })
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
    match response.status {
        401 | 403 => Error::Auth,
        400 if content_policy(&response.body) => Error::ContentPolicy,
        400 | 422 => Error::Payload,
        402 => Error::Quota,
        404 => Error::UnsupportedVoice,
        408 => Error::Timeout,
        409 => Error::Invalid("provider rejected request"),
        413 => Error::Payload,
        429 => Error::RateLimit {
            retry_after_seconds: retry_after(&response.headers),
        },
        500..=599 => Error::Server,
        300..=399 => Error::Invalid("provider redirect"),
        _ => Error::Server,
    }
}

fn content_policy(body: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return false;
    };
    let detail = value.get("detail");
    ["status", "code", "type"]
        .iter()
        .filter_map(|key| detail.and_then(|detail| detail.get(*key)))
        .filter_map(|value| value.as_str())
        .any(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("content") || value.contains("moderation") || value.contains("safety")
        })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
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
