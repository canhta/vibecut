//! OpenAI providers: consent-gated HTTPS transport, typed failures, text-only file
//! recognition, contextual translation and Chat Completions speech synthesis.
mod speech;
mod translation;
use crate::contracts::checkpoint;
use crate::remote::{self, UploadDisclosure};
use crate::{
    AudioWindow, Capabilities, CostEstimate, Error, Evidence, ExecutionHooks, Operation,
    ProviderConfig, RecognitionSegment, Request, SampleRange, Scope, Stage, Support, Transcriber,
};
use captions::{LanguageTag, NfcText};
use serde::Deserialize;
pub use speech::*;
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;
pub use translation::*;

use crate::transport::{
    ApiKey, HttpRequest, HttpResponse, HttpTransport, MAX_RESPONSE_BYTES, Multipart, wav,
};

/// The only production model admitted here. Deprecated transcription models are refused.
/// Provider backend name expected in the tagged remote configuration.
pub(crate) const BACKEND: &str = "openai";
/// Current documented file limit for `/v1/audio/transcriptions` (25 MB decimal).
pub const MAX_UPLOAD_BYTES: u64 = 25_000_000;
/// The only admitted provider base URL in v1; no proxies or custom processors.
pub(crate) const BASE_URL: &str = "https://api.openai.com/v1";
/// The only admitted transcription endpoint.
pub const ENDPOINT: &str = "https://api.openai.com/v1/audio/transcriptions";
/// Published list price, integer micro-USD per audio second ($0.0045/min).
const POLICY_URL: &str = "https://developers.openai.com/api/docs/guides/your-data";
const DOCS_URL: &str = "https://developers.openai.com/api/docs/guides/speech-to-text";
/// File mode is the only mode this adapter advertises.
pub const MODE: &str = "file";
/// Product ASR languages. Extend this data table only with documented API support.
pub const LANGUAGES: [&str; 3] = ["vi", "en", "zh"];
/// Settings identity for composition-owned cache keys.
const SETTINGS_FINGERPRINT: &str = "openai-transport-v1;format=json;wav=pcm16;languages=vi,en,zh";

/// Consent-gated OpenAI file transcription over an injected blocking transport.
pub struct OpenAiTranscriber {
    config: ProviderConfig,
    key: ApiKey,
    transport: Arc<dyn HttpTransport>,
    languages: Vec<LanguageTag>,
}
impl OpenAiTranscriber {
    /// The one admitted configuration for this model id.
    pub fn provider_config(model: &str) -> Result<ProviderConfig, Error> {
        remote::provider_config(
            model,
            SETTINGS_FINGERPRINT,
            BACKEND,
            BASE_URL,
            "openai",
            "OPENAI_API_KEY",
        )
    }

    /// Admits a bounded selected model with the `openai` backend, this
    /// adapter's settings fingerprint and a remote versioned HTTPS base URL. The
    /// key is resolved by composition, never here.
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
            Some("OpenAI requires remote config"),
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
        remote::rounded_up_seconds(scope).map_or(CostEstimate::Unknown, |seconds| {
            crate::defaults::audio_estimate(&self.config, seconds)
        })
    }

    fn disclosure(&self, scope: &Scope) -> Result<Option<UploadDisclosure>, Error> {
        let capabilities = self.capabilities(scope);
        remote::validate_audio_scope(&self.config, scope, &capabilities)?;
        Ok(Some(remote::disclosure(remote::Disclosure {
            operation: Operation::Transcribe,
            provider: "OpenAI",
            model: self.config.model(),
            languages: NfcText::from(scope.source_language.as_str()),
            audio_micros: remote::duration_micros(scope)?,
            upload_bytes: remote::encoded_bytes(scope, MAX_UPLOAD_BYTES)?,
            estimate: self.estimate(scope),
            links: &[("policy", POLICY_URL), ("docs", DOCS_URL)],
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
        let segments = parse_transcription(&response, audio.range)?;
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
    form.field("model", scope.provider.model().as_str());
    form.field("response_format", "json");
    form.field("languages[]", scope.source_language.as_str());
    form.file("audio.wav", "audio/wav", &audio);
    let (content_type, body) = form.finish();
    Ok(HttpRequest {
        method: "POST",
        url: ENDPOINT.into(),
        headers: vec![
            ("Authorization".into(), format!("Bearer {}", key.expose())),
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

pub(crate) fn status_error(response: &HttpResponse) -> Error {
    remote::status_error(response, |status, fields| {
        (status == 429 && remote::quota(fields)).then_some(Error::Quota)
    })
}
