//! Blocking speech synthesis policy. Engine owns retries, scheduling and placement.
use crate::{Error, ExecutionHooks, MAX_PCM_BYTES, Money, Operation, ProviderConfig, Support};
use captions::{LanguageTag, NfcText, NormalizedText};
use sha2::{Digest, Sha256};

/// Independent input ceiling; provider limits can be smaller.
pub const MAX_SPEECH_TEXT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeechFormat {
    WavPcm16,
    Pcm16,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceCategory {
    BuiltIn,
    Stock,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeechVoice {
    pub id: NfcText,
    pub category: VoiceCategory,
}
/// Exact language/script pair. No implicit locale or script fallback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeechLanguage {
    pub language: LanguageTag,
    /// ISO 15924 script, e.g. Latn, Hans or Hant.
    pub script: NfcText,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpeechOutput {
    pub format: SpeechFormat,
    pub sample_rate: u32,
}
/// Disclosure text is provider-owned and consent-bound, not a promise by the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeechPolicy {
    pub policy_url: NfcText,
    pub retention: NfcText,
    pub training: NfcText,
    pub voice_disclosure: NfcText,
}
/// Resolved, IO-free data for the exact configuration, voice and language query.
/// None limits mean unknown; they never mean unlimited. Engine schedules concurrency.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeechCapabilities {
    pub provider: ProviderConfig,
    /// Includes model, voice, language, format, instructions and normalization settings.
    pub fingerprint: NfcText,
    pub languages: Vec<SpeechLanguage>,
    pub voices: Vec<SpeechVoice>,
    pub outputs: Vec<SpeechOutput>,
    /// Unicode extended graphemes; scalar counts for provider billing are separate.
    pub max_characters: Option<u64>,
    pub streaming: Support,
    pub max_concurrency: Option<u32>,
    pub requests_per_minute: Option<u32>,
    pub cost_model: NfcText,
    pub policy: SpeechPolicy,
}

/// Exact immutable NFC request. No source-media window or Project is involved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeechRequest {
    provider: ProviderConfig,
    text: NfcText,
    voice: NfcText,
    language: SpeechLanguage,
    output: SpeechOutput,
    max_duration_micros: u64,
    characters: u64,
    text_sha256: [u8; 32],
}
impl SpeechRequest {
    /// Duration is a positive ceiling in integer microseconds. Output is mono.
    pub fn new(
        provider: ProviderConfig,
        text: NfcText,
        voice: NfcText,
        language: SpeechLanguage,
        output: SpeechOutput,
        max_duration_micros: u64,
    ) -> Result<Self, Error> {
        if text.as_str().len() > MAX_SPEECH_TEXT_BYTES {
            return Err(Error::Payload);
        }
        crate::contracts::policy::nonempty(&text)?;
        crate::contracts::policy::nonempty(&voice)?;
        let script = language.script.as_str();
        if script.len() != 4 || !script.bytes().all(|b| b.is_ascii_alphabetic()) {
            return Err(Error::Invalid("speech script"));
        }
        if output.sample_rate == 0 || max_duration_micros == 0 {
            return Err(Error::Invalid("speech rate or duration"));
        }
        let characters = NormalizedText::new(text.as_str())?.spans().len() as u64;
        let text_sha256 = Sha256::digest(text.as_str().as_bytes()).into();
        Ok(Self {
            provider,
            text,
            voice,
            language,
            output,
            max_duration_micros,
            characters,
            text_sha256,
        })
    }
    pub fn provider(&self) -> &ProviderConfig {
        &self.provider
    }
    pub fn operation(&self) -> Operation {
        Operation::Synthesize
    }
    pub fn text(&self) -> &NfcText {
        &self.text
    }
    pub fn voice(&self) -> &NfcText {
        &self.voice
    }
    pub fn language(&self) -> &SpeechLanguage {
        &self.language
    }
    pub fn output(&self) -> SpeechOutput {
        self.output
    }
    pub fn max_duration_micros(&self) -> u64 {
        self.max_duration_micros
    }
    pub fn characters(&self) -> u64 {
        self.characters
    }
    pub fn text_sha256(&self) -> [u8; 32] {
        self.text_sha256
    }
}
/// Per-invocation price. Unknown is explicit so an unresolved quote fails closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SynthesisEstimate {
    Free,
    Exact(Money),
    UpperBound(Money),
    Unknown,
}
impl SynthesisEstimate {
    fn covers(&self, cost: &Money) -> bool {
        match self {
            Self::Free => cost.micros() == 0,
            Self::Exact(bound) | Self::UpperBound(bound) => covers(bound, cost),
            Self::Unknown => false,
        }
    }
}
fn covers(bound: &Money, cost: &Money) -> bool {
    bound.currency() == cost.currency() && cost.micros() <= bound.micros()
}
/// In-memory approval, never persisted. Equality binds full text as well as its hash,
/// exact provider identity/settings, policy, operation, capabilities and quote.
#[derive(Clone, Debug)]
pub struct SynthesisConsent {
    request: SpeechRequest,
    capabilities: SpeechCapabilities,
    estimate: SynthesisEstimate,
    budget: Money,
}
impl SynthesisConsent {
    /// Call only after affirmative approval of the exact payload and disclosure.
    /// Budget is per invocation; engine must reserve aggregate retry/chunk spending.
    pub fn grant(
        request: &SpeechRequest,
        capabilities: &SpeechCapabilities,
        estimate: SynthesisEstimate,
        budget: Money,
    ) -> Result<Self, Error> {
        capabilities.check(request)?;
        check_budget(&estimate, &budget)?;
        Ok(Self {
            request: request.clone(),
            capabilities: capabilities.clone(),
            estimate,
            budget,
        })
    }
}
fn check_budget(estimate: &SynthesisEstimate, budget: &Money) -> Result<(), Error> {
    match estimate {
        SynthesisEstimate::Free => Ok(()),
        SynthesisEstimate::Exact(cost) | SynthesisEstimate::UpperBound(cost)
            if covers(budget, cost) =>
        {
            Ok(())
        }
        _ => Err(Error::Cost),
    }
}
impl SpeechCapabilities {
    pub fn check(&self, request: &SpeechRequest) -> Result<(), Error> {
        if self.provider != request.provider {
            return Err(Error::Unsupported);
        }
        if !self.languages.contains(&request.language) {
            return Err(Error::UnsupportedLanguage);
        }
        if !self.voices.iter().any(|voice| voice.id == request.voice) {
            return Err(Error::UnsupportedVoice);
        }
        if !self.outputs.contains(&request.output) {
            return Err(Error::Unsupported);
        }
        if self
            .max_characters
            .is_some_and(|max| request.characters > max)
        {
            return Err(Error::Payload);
        }
        if self.max_concurrency == Some(0) || self.requests_per_minute == Some(0) {
            return Err(Error::Unsupported);
        }
        for value in [
            &self.fingerprint,
            &self.cost_model,
            &self.policy.retention,
            &self.policy.training,
            &self.policy.voice_disclosure,
        ] {
            crate::contracts::policy::nonempty(value)?;
        }
        let url = url::Url::parse(self.policy.policy_url.as_str())
            .map_err(|_| Error::Invalid("speech policy URL"))?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(Error::Invalid("speech policy URL"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeechUsage {
    /// Provider-reported billable characters, which may count scalars instead of graphemes.
    pub billable_characters: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub actual_cost: Money,
}
/// Adapter transport record. The execution seam validates this before delivery.
/// Adapters must bound decoding/allocation by MAX_PCM_BYTES and request duration.
#[derive(Clone)]
pub struct SpeechAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    /// Sample-derived duration rounded up to the nearest microsecond.
    pub duration_micros: u64,
    pub fingerprint: NfcText,
    pub usage: SpeechUsage,
    pub transcript: Option<NfcText>,
}
impl std::fmt::Debug for SpeechAudio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpeechAudio")
            .field("sample_count", &self.samples.len())
            .field("sample_rate", &self.sample_rate)
            .field("duration_micros", &self.duration_micros)
            .finish_non_exhaustive()
    }
}
/// Object-safe, single-attempt adapter. config/capabilities/estimate perform no IO.
/// Synthesize must call prepare_synthesis before IO even when invoked directly,
/// check cancellation during blocking work, and cap response bytes and decoding.
pub trait SpeechSynthesizer: Send + Sync {
    fn config(&self) -> &ProviderConfig;
    fn capabilities(&self, request: &SpeechRequest) -> SpeechCapabilities;
    fn estimate(&self, request: &SpeechRequest) -> SynthesisEstimate;
    /// Provider-neutral upload disclosure for this exact request; a local adapter
    /// returns `Ok(None)`.
    fn disclosure(
        &self,
        _request: &SpeechRequest,
    ) -> Result<Option<crate::UploadDisclosure>, Error> {
        Ok(None)
    }
    fn synthesize(
        &self,
        request: &SpeechRequest,
        consent: &SynthesisConsent,
        hooks: &dyn ExecutionHooks,
    ) -> Result<SpeechAudio, Error>;
}
/// Shared preflight, also available to adapters that expose direct execution.
/// A changed quote (even smaller), policy or scope requires new approval.
pub fn prepare_synthesis(
    provider: &dyn SpeechSynthesizer,
    request: &SpeechRequest,
    consent: &SynthesisConsent,
) -> Result<(), Error> {
    if provider.config() != request.provider() {
        return Err(Error::ConsentRequired);
    }
    let caps = provider.capabilities(request);
    caps.check(request)?;
    let estimate = provider.estimate(request);
    if consent.request != *request || consent.capabilities != caps || consent.estimate != estimate {
        return Err(Error::ConsentRequired);
    }
    check_budget(&estimate, &consent.budget)
}
/// Production execution seam: no fallback, retries, partial audio or Project edits.
/// Cancellation before/after IO discards the entire result. Hooks have no speech
/// stage yet; the engine's narration integration owns adding job stage routing.
pub fn synthesize(
    provider: &dyn SpeechSynthesizer,
    request: &SpeechRequest,
    consent: &SynthesisConsent,
    hooks: &dyn ExecutionHooks,
) -> Result<SpeechAudio, Error> {
    if hooks.cancelled() {
        return Err(Error::Cancelled);
    }
    prepare_synthesis(provider, request, consent)?;
    if hooks.cancelled() {
        return Err(Error::Cancelled);
    }
    let outcome = provider.synthesize(request, consent, hooks);
    if hooks.cancelled() {
        return Err(Error::Cancelled);
    }
    let audio = outcome?;
    validate_speech_audio(&audio, request, &consent.capabilities, &consent.estimate)?;
    if hooks.cancelled() {
        return Err(Error::Cancelled);
    }
    Ok(audio)
}
/// Validate decoded mono samples, exact identity, transcript and actual spending.
/// Wire truncation/container integrity must additionally be checked by the adapter.
pub fn validate_speech_audio(
    audio: &SpeechAudio,
    request: &SpeechRequest,
    capabilities: &SpeechCapabilities,
    estimate: &SynthesisEstimate,
) -> Result<(), Error> {
    capabilities.check(request)?;
    let count = audio.samples.len() as u128;
    if audio.sample_rate == 0 {
        return Err(Error::OutputRejected);
    }
    let duration_micros = (count * 1_000_000).div_ceil(u128::from(audio.sample_rate));
    if count == 0
        || count * 4 > u128::from(MAX_PCM_BYTES)
        || audio.sample_rate != request.output.sample_rate
        || audio.fingerprint != capabilities.fingerprint
        || audio.samples.iter().any(|v| !v.is_finite())
        || count * 1_000_000
            > u128::from(request.max_duration_micros) * u128::from(audio.sample_rate)
        || duration_micros != u128::from(audio.duration_micros)
        || audio
            .transcript
            .as_ref()
            .is_some_and(|t| t != request.text())
    {
        return Err(Error::OutputRejected);
    }
    if !estimate.covers(&audio.usage.actual_cost) {
        return Err(Error::Cost);
    }
    Ok(())
}
