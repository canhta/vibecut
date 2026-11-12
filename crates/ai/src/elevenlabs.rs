//! ElevenLabs provider adapters. Scribe transcription lives here; speech synthesis
//! is in `speech`.
pub mod speech;
use crate::contracts::checkpoint;
use crate::remote::{self, UploadDisclosure};
use crate::transport::{ApiKey, Multipart, wav};
use crate::{
    AudioWindow, Capabilities, CostEstimate, Error, Evidence, ExecutionHooks, Operation,
    Provenance, ProviderConfig, RawToken, RecognitionSegment, Request, SampleRange, Scope, Stage,
    Support, TimingUnit, Transcriber,
};
use ::transport::{HttpRequest, HttpResponse, HttpTransport, MAX_RESPONSE_BYTES};
use captions::{LanguageTag, NfcText};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

const BACKEND: &str = "elevenlabs";
const BASE_URL: &str = "https://api.elevenlabs.io/v1";
pub const ENDPOINT: &str = "https://api.elevenlabs.io/v1/speech-to-text";
/// Documented standard-mode file ceiling; duration is separately capped at 10 h.
pub const MAX_UPLOAD_BYTES: u64 = 3_000_000_000;
pub const MAX_DURATION_MICROS: u64 = 10 * 60 * 60 * 1_000_000;
/// Published list price, integer micro-USD per audio hour ($0.22/h).
pub const MODE: &str = "file";
pub const LANGUAGES: [&str; 3] = ["vi", "en", "zh"];
const SETTINGS_FINGERPRINT: &str = "elevenlabs-transport-v1;format=json;wav=pcm16;timestamps=word;languages=vi,en,zh;logging=default";
const POLICY_URL: &str = "https://elevenlabs.io/privacy-policy";
const DOCS_URL: &str = "https://elevenlabs.io/docs/capabilities/speech-to-text";
const PRICING_URL: &str = "https://elevenlabs.io/pricing/api?price.section=speech_to_text";

pub struct ElevenLabsTranscriber {
    config: ProviderConfig,
    key: ApiKey,
    transport: Arc<dyn HttpTransport>,
    languages: Vec<LanguageTag>,
}
impl ElevenLabsTranscriber {
    /// The one admitted configuration for this model id.
    pub fn provider_config(model: &str) -> Result<ProviderConfig, Error> {
        remote::provider_config(
            model,
            SETTINGS_FINGERPRINT,
            BACKEND,
            BASE_URL,
            "elevenlabs",
            "ELEVENLABS_API_KEY",
        )
    }

    /// Admits a bounded selected model, backend `elevenlabs`, this adapter's settings
    /// fingerprint and the official HTTPS base URL.
    pub fn new(
        config: ProviderConfig,
        key: ApiKey,
        transport: Arc<dyn HttpTransport>,
    ) -> Result<Self, Error> {
        remote::admit(
            &config,
            Some(SETTINGS_FINGERPRINT),
            BACKEND,
            BASE_URL,
            Some("ElevenLabs requires remote config"),
        )?;
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
        remote::rounded_up_seconds(scope).map_or(CostEstimate::Unknown, |seconds| {
            crate::defaults::audio_estimate(&self.config, seconds)
        })
    }

    fn disclosure(&self, scope: &Scope) -> Result<Option<UploadDisclosure>, Error> {
        let capabilities = self.capabilities(scope);
        remote::validate_audio_scope(&self.config, scope, &capabilities)?;
        Ok(Some(remote::disclosure(remote::Disclosure {
            operation: Operation::Transcribe,
            provider: "ElevenLabs",
            model: self.config.model(),
            languages: NfcText::from(scope.source_language.as_str()),
            audio_micros: remote::duration_micros(scope)?,
            upload_bytes: remote::encoded_bytes(scope, MAX_UPLOAD_BYTES)?,
            estimate: self.estimate(scope),
            links: &[
                ("policy", POLICY_URL),
                ("pricing", PRICING_URL),
                ("docs", DOCS_URL),
            ],
        })))
    }

    fn transcribe(
        &self,
        audio: AudioWindow<'_>,
        request: &Request<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<RecognitionSegment>, Error> {
        checkpoint(hooks, Stage::Transcribe)?;
        let scope = request.scope;
        let capabilities = self.capabilities(scope);
        remote::validate_audio_scope(&self.config, scope, &capabilities)?;
        audio.validate(scope)?;
        if remote::encoded_bytes(scope, MAX_UPLOAD_BYTES)? > scope.max_bytes {
            return Err(Error::Payload);
        }
        request.prepare(&capabilities, &self.estimate(scope), Operation::Transcribe)?;
        let outbound = build_request(scope, &self.key, audio)?;
        checkpoint(hooks, Stage::Transcribe)?;
        let response = self.transport.send(outbound)?;
        remote::check_model_response(&response, &self.config)?;
        checkpoint(hooks, Stage::Transcribe)?;
        let segments = parse_transcription(&response, audio.range, audio.sample_rate)?;
        checkpoint(hooks, Stage::Transcribe)?;
        Ok(segments)
    }
}

fn build_request(
    scope: &Scope,
    key: &ApiKey,
    audio: AudioWindow<'_>,
) -> Result<HttpRequest, Error> {
    let audio = wav(&audio)?;
    if u64::try_from(audio.len()).map_err(|_| Error::Payload)? > MAX_UPLOAD_BYTES {
        return Err(Error::Payload);
    }
    let mut form = Multipart::new(&audio)?;
    form.field("model_id", scope.provider.model().as_str());
    form.field("language_code", scope.source_language.as_str());
    form.field("timestamps_granularity", "word");
    form.file("audio.wav", "audio/wav", &audio);
    let (content_type, body) = form.finish();
    Ok(HttpRequest {
        method: "POST",
        url: ENDPOINT.into(),
        headers: vec![
            ("xi-api-key".into(), key.expose().to_owned()),
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
        serde_json::from_slice(&response.body).map_err(|_| Error::Schema)?;
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
    let raw = String::from_utf8(raw).map_err(|_| Error::Schema)?;
    let text = NfcText::from(raw);
    if text != NfcText::from(parsed.text) {
        return Err(Error::Schema);
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
        return Err(Error::Schema);
    }
    let scaled = seconds * f64::from(sample_rate);
    if !scaled.is_finite() || scaled > u64::MAX as f64 {
        return Err(Error::Payload);
    }
    let relative = scaled.round();
    if relative > (window.len() as f64) + 1.0 {
        return Err(Error::Schema);
    }
    let relative = relative.max(0.0) as u64;
    Ok(window.start().saturating_add(relative).min(window.end()))
}

fn status_error(response: &HttpResponse) -> Error {
    remote::status_error(response, |status, fields| {
        if status == 402 {
            return Some(Error::Quota);
        }
        if matches!(status, 400 | 422) {
            if remote::matches(fields, &["invalid_api_key", "authentication"]) {
                return Some(Error::Auth);
            }
            if remote::matches(fields, &["file_too_large", "too_large", "payload"]) {
                return Some(Error::Payload);
            }
        }
        None
    })
}
