//! Classifies filler words in an ordered transcript through one admitted text model.
//!
//! The lexicon detector stays local and synchronous. This owner sends the exact
//! bounded transcript and returns only exact word IDs the model chose; the engine
//! maps those IDs back to its visible measured word spans.
mod remote;
use crate::{
    Capabilities, CostEstimate, Error, ExecutionHooks, Operation, ProviderConfig, Request, Scope,
    Stage,
};
use captions::{FillerKind, LanguageTag, NfcText};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub use remote::{AnthropicFillerClassifier, OPENAI_MODEL, OpenAiFillerClassifier};

pub const MODE: &str = "transcript-fillers";
pub const MAX_SEGMENTS: usize = 10_000;
pub const MAX_WORDS: usize = 50_000;
pub const MAX_INPUT_BYTES: usize = 96 * 1024;
pub const MAX_WIRE: u64 = 131_072;
pub const OPENAI_SETTINGS: &str = "fillers-openai-v1;responses;strict;store=false;max_output=8192";
pub const ANTHROPIC_SETTINGS: &str = "fillers-anthropic-v1;messages;json_schema;max_output=8192";

/// Typed reason a filler transcript or model result is refused before use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FillerRefusal {
    #[error("transcript has no timed words")]
    EmptyTranscript,
    #[error("transcript language is unsupported")]
    UnsupportedLanguage,
    #[error("filler transcript segments or word IDs are invalid")]
    InvalidSegments,
    #[error("filler payload exceeds its limit")]
    Payload,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FillerSettings {
    /// Transcript language shared by every segment.
    pub language: LanguageTag,
    /// Whether discourse markers may be classified in addition to filled pauses.
    pub include_markers: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FillerWord {
    /// The engine's stable candidate ID for this measured word.
    pub id: NfcText,
    pub text: NfcText,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FillerSegment {
    /// Stable `<caption item>:<cue>` identity; never a timestamp.
    pub id: NfcText,
    pub text: NfcText,
    pub words: Vec<FillerWord>,
}
/// Ordered measured words of one language with the requested marker policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FillerInput {
    pub segments: Vec<FillerSegment>,
    pub settings: FillerSettings,
}
impl FillerInput {
    /// Refuse empty, unsupported or oversized transcripts before consent.
    pub fn validate(&self) -> Result<(), FillerRefusal> {
        if !crate::supports_text_language(&self.settings.language) {
            return Err(FillerRefusal::UnsupportedLanguage);
        }
        if self.segments.is_empty() {
            return Err(FillerRefusal::EmptyTranscript);
        }
        if self.segments.len() > MAX_SEGMENTS {
            return Err(FillerRefusal::Payload);
        }
        let mut segment_ids = BTreeSet::new();
        let mut word_ids = BTreeSet::new();
        let mut words = 0usize;
        let mut bytes = 0usize;
        for segment in &self.segments {
            if segment.id.as_str().trim().is_empty()
                || segment.id.as_str().len() > 256
                || segment.text.as_str().trim().is_empty()
                || segment.words.is_empty()
                || !segment_ids.insert(&segment.id)
            {
                return Err(FillerRefusal::InvalidSegments);
            }
            bytes = bytes
                .saturating_add(segment.id.as_str().len())
                .saturating_add(segment.text.as_str().len());
            for word in &segment.words {
                if word.id.as_str().trim().is_empty()
                    || word.id.as_str().len() > 256
                    || word.text.as_str().trim().is_empty()
                    || word.text.as_str().len() > 256
                    || !word_ids.insert(&word.id)
                {
                    return Err(FillerRefusal::InvalidSegments);
                }
                bytes = bytes
                    .saturating_add(word.id.as_str().len())
                    .saturating_add(word.text.as_str().len());
            }
            words += segment.words.len();
        }
        if words > MAX_WORDS || bytes > MAX_INPUT_BYTES {
            return Err(FillerRefusal::Payload);
        }
        Ok(())
    }
    /// Exact complete input bound to this invocation's consent.
    pub fn consent_text(&self) -> Result<NfcText, Error> {
        self.validate()?;
        let text = serde_json::to_string(self).map_err(|_| Error::Payload)?;
        if text.len() > MAX_INPUT_BYTES {
            return Err(FillerRefusal::Payload.into());
        }
        Ok(text.into())
    }
    /// Resolve returned word IDs against this exact transcript; unknown or
    /// duplicate IDs never disappear silently.
    pub fn resolve(&self, proposal: FillerProposal) -> Result<Vec<FillerFinding>, FillerRefusal> {
        self.validate()?;
        let ids: BTreeSet<_> = self
            .segments
            .iter()
            .flat_map(|segment| segment.words.iter().map(|word| &word.id))
            .collect();
        let mut returned = BTreeSet::new();
        let mut findings = Vec::with_capacity(proposal.fillers.len());
        for finding in proposal.fillers {
            if !ids.contains(&finding.id)
                || !returned.insert(finding.id.clone())
                || (finding.kind == FillerKind::DiscourseMarker && !self.settings.include_markers)
            {
                return Err(FillerRefusal::InvalidSegments);
            }
            findings.push(finding);
        }
        Ok(findings)
    }
}

/// One word the model classified and the kind it assigned.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FillerFinding {
    pub id: NfcText,
    pub kind: FillerKind,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FillerProposal {
    pub fillers: Vec<FillerFinding>,
}

/// Blocking single-attempt provider seam; adapters never fall back or retry.
pub trait FillerClassifier: Send + Sync {
    fn config(&self) -> &ProviderConfig;
    fn capabilities(&self, scope: &Scope) -> Capabilities;
    fn estimate(&self, scope: &Scope) -> CostEstimate;
    /// Provider-neutral upload disclosure for this exact scope; a local adapter
    /// returns `Ok(None)`.
    fn disclosure(&self, _scope: &Scope) -> Result<Option<crate::UploadDisclosure>, Error> {
        Ok(None)
    }
    fn classify(
        &self,
        input: &FillerInput,
        request: &Request<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<FillerProposal, Error>;
}
pub(super) fn prepare(
    provider: &dyn FillerClassifier,
    input: &FillerInput,
    request: &Request<'_>,
) -> Result<(), Error> {
    if provider.config() != &request.scope.provider {
        return Err(Error::Unsupported);
    }
    if request.scope.source_language != input.settings.language
        || request.scope.target_language.is_some()
        || request.scope.texts != [input.consent_text()?]
    {
        return Err(Error::ConsentRequired);
    }
    if input.consent_text()?.as_str().len() as u64 > request.scope.max_bytes {
        return Err(Error::Payload);
    }
    request.prepare(
        &provider.capabilities(request.scope),
        &provider.estimate(request.scope),
        Operation::ClassifyFillers,
    )
}
/// Validate consent and cost, execute one provider invocation and resolve its IDs.
pub fn classify_fillers(
    provider: &dyn FillerClassifier,
    input: &FillerInput,
    request: &Request<'_>,
    hooks: &dyn ExecutionHooks,
) -> Result<Vec<FillerFinding>, Error> {
    crate::contracts::checkpoint(hooks, Stage::ClassifyFillers)?;
    prepare(provider, input, request)?;
    let proposal = provider.classify(input, request, hooks)?;
    crate::contracts::checkpoint(hooks, Stage::ClassifyFillers)?;
    Ok(input.resolve(proposal)?)
}
