//! Provider request contracts.
use crate::{Consent, CostEstimate, Error, ProviderConfig, Scope, authorize};
use captions::{LanguageTag, NfcText};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Transcribe,
    Align,
    Translate,
    SuggestClips,
    GenerateChapters,
    Synthesize,
}
/// Absolute source-audio sample coordinates, half-open and nonnegative.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "[u64; 2]", into = "[u64; 2]")]
pub struct SampleRange {
    start: u64,
    end: u64,
}
impl SampleRange {
    pub fn new(start: u64, end: u64) -> Result<Self, Error> {
        if end <= start {
            return Err(Error::Invalid("empty or reversed sample range"));
        }
        Ok(Self { start, end })
    }
    pub fn start(self) -> u64 {
        self.start
    }
    pub fn end(self) -> u64 {
        self.end
    }
    pub fn len(self) -> u64 {
        self.end - self.start
    }
    pub fn is_empty(self) -> bool {
        false
    }
    pub fn contains(self, other: Self) -> bool {
        self.start <= other.start && self.end >= other.end
    }
}
impl TryFrom<[u64; 2]> for SampleRange {
    type Error = Error;
    fn try_from([start, end]: [u64; 2]) -> Result<Self, Error> {
        Self::new(start, end)
    }
}
impl From<SampleRange> for [u64; 2] {
    fn from(value: SampleRange) -> Self {
        [value.start, value.end]
    }
}

pub use captions::TimingUnit;
/// Conditional/unknown features are refused until the adapter resolves the condition
/// for the exact request. Capability presence does not establish timing accuracy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "condition", rename_all = "snake_case")]
pub enum Support {
    Supported,
    Unsupported,
    Unknown,
    Conditional(NfcText),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    Advertised,
    FixtureVerified,
    LiveVerified,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Capabilities {
    pub model: NfcText,
    pub mode: NfcText,
    pub operation: Operation,
    /// Exact BCP 47 tags; no implicit locale/script fallback.
    pub languages: Vec<LanguageTag>,
    pub target_languages: Vec<LanguageTag>,
    pub timing: BTreeSet<TimingUnit>,
    pub diarization: Support,
    pub live_audio_input: Support,
    pub file_result_stream: Support,
    pub max_upload_bytes: Option<u64>,
    pub max_duration_micros: Option<u64>,
    pub evidence: Evidence,
    pub revision: NfcText,
}
impl Capabilities {
    pub fn check(&self, scope: &Scope) -> Result<(), Error> {
        scope.validate()?;
        if self.model != *scope.provider.model()
            || self.mode != scope.mode
            || self.operation != scope.operation
        {
            return Err(Error::Unsupported);
        }
        if !self.languages.contains(&scope.source_language)
            || scope
                .target_language
                .as_ref()
                .is_some_and(|l| !self.target_languages.contains(l))
        {
            return Err(Error::UnsupportedLanguage);
        }
        crate::policy::nonempty(&self.revision)?;
        if self
            .max_upload_bytes
            .is_some_and(|max| scope.max_bytes > max)
        {
            return Err(Error::Payload);
        }
        let duration = u128::from(scope.source_samples.len()) * 1_000_000;
        if self
            .max_duration_micros
            .is_some_and(|max| duration > u128::from(max) * u128::from(scope.sample_rate))
        {
            return Err(Error::Payload);
        }
        Ok(())
    }
}
/// Engine owns scheduling/lifecycle. Hooks carry only bounded stage metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Transcribe,
    Align,
    Translate,
    SuggestClips,
    GenerateChapters,
    Vision,
    Assemble,
}
/// Hooks may run on an inference observer thread and must return promptly.
/// Implementations synchronize their own job state; this seam owns no job.
pub trait ExecutionHooks: Send + Sync {
    fn cancelled(&self) -> bool;
    fn stage(&self, stage: Stage);
    /// Bounded native progress in percent; default preserves stage-only consumers.
    fn progress(&self, _stage: Stage, _percent: u8) {}
    /// Native work has observed cancellation. The job owner can enter Cancelling
    /// immediately while the current native operation drains; no result is returned.
    fn cancelling(&self) {}
}
pub(crate) fn checkpoint(hooks: &dyn ExecutionHooks, stage: Stage) -> Result<(), Error> {
    if hooks.cancelled() {
        return Err(Error::Cancelled);
    }
    hooks.stage(stage);
    if hooks.cancelled() {
        return Err(Error::Cancelled);
    }
    Ok(())
}
/// Per-invocation f32 PCM safety ceiling, independent of the wire-upload ceiling.
/// 64 MiB admits a maximum advertised 25 MB PCM16 upload (~50 MB decoded) with
/// margin. Adapters that upload derive their own encoded byte limit from
/// `Scope::max_bytes` so a valid container is not rejected by its larger decoded form.
pub const MAX_PCM_BYTES: u64 = 64 * 1024 * 1024;
/// Immutable mono PCM view. Offset is already in source samples; no second offset
/// is applied by assembly. Decode/resample and content hashing belong to callers.
/// Validation uses `MAX_PCM_BYTES`, never the wire-upload ceiling.
#[derive(Clone, Copy)]
pub struct AudioWindow<'a> {
    pub samples: &'a [f32],
    pub range: SampleRange,
    pub sample_rate: u32,
}
impl std::fmt::Debug for AudioWindow<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioWindow")
            .field("range", &self.range)
            .field("sample_rate", &self.sample_rate)
            .finish_non_exhaustive()
    }
}
impl AudioWindow<'_> {
    pub(crate) fn validate(&self, scope: &Scope) -> Result<(), Error> {
        if self.sample_rate == 0
            || self.sample_rate != scope.sample_rate
            || self.range != scope.source_samples
            || u64::try_from(self.samples.len()).ok() != Some(self.range.len())
            || self.samples.iter().any(|v| !v.is_finite())
        {
            return Err(Error::Invalid("audio window"));
        }
        let bytes = self.range.len().checked_mul(4).ok_or(Error::Payload)?;
        if bytes > MAX_PCM_BYTES {
            return Err(Error::Payload);
        }
        Ok(())
    }
}
/// Shared request policy; config is included in scope so model/endpoint changes
/// invalidate approval. Estimates cover this invocation, not the whole engine job.
#[derive(Clone, Debug)]
pub struct Request<'a> {
    pub scope: &'a Scope,
    pub consent: Option<&'a Consent>,
}
impl Request<'_> {
    pub(crate) fn prepare(
        &self,
        caps: &Capabilities,
        estimate: &CostEstimate,
        operation: Operation,
    ) -> Result<(), Error> {
        if self.scope.operation != operation {
            return Err(Error::Unsupported);
        }
        self.scope.validate()?;
        caps.check(self.scope)?;
        authorize(self.scope, estimate, self.consent)
    }
}
/// Object-safe blocking adapter seam; engine runs it in a cancellable worker.
/// Adapters must check hooks during long work and return only safe typed failures.
pub trait Transcriber: Send + Sync {
    fn config(&self) -> &ProviderConfig;
    fn capabilities(&self, scope: &Scope) -> Capabilities;
    fn estimate(&self, scope: &Scope) -> CostEstimate;
    fn transcribe(
        &self,
        audio: AudioWindow<'_>,
        request: &Request<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<crate::RecognitionSegment>, Error>;
}
pub trait Aligner: Send + Sync {
    fn config(&self) -> &ProviderConfig;
    fn capabilities(&self, scope: &Scope) -> Capabilities;
    fn estimate(&self, scope: &Scope) -> CostEstimate;
    /// Return ranges for the supplied missing spans only, in identical order.
    fn align(
        &self,
        audio: AudioWindow<'_>,
        missing: &[crate::RecognitionSegment],
        request: &Request<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<crate::RecognitionSegment>, Error>;
}
