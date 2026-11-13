//! Chooses caption keyword emphasis in an ordered transcript through one admitted text model.
//!
//! The lexicon selector in `captions` stays local and synchronous. This owner sends the exact
//! bounded cues and returns only word positions the model chose inside each cue; the engine maps
//! them back to the cue's verified word records. A failed provider is a typed failure: nothing
//! here or in the engine substitutes the lexicon.
mod remote;
use crate::{
    Capabilities, CostEstimate, Error, ExecutionHooks, Operation, ProviderConfig, Request, Scope,
    Stage,
};
use captions::{LanguageTag, NfcText};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub use remote::{AnthropicEmphasisSelector, OpenAiEmphasisSelector};

pub const MODE: &str = "caption-emphasis";
impl Scope {
    /// Consent scope of one emphasis selection, bound to the complete cue payload.
    pub fn emphasis(
        provider: ProviderConfig,
        input: &EmphasisInput,
        payload_fingerprint: NfcText,
    ) -> Result<Self, Error> {
        Self::text(
            provider,
            Operation::SelectEmphasis,
            MODE,
            input.consent_text()?,
            payload_fingerprint,
            input.settings.language.clone(),
            MAX_WIRE,
        )
    }
}
pub const MAX_SEGMENTS: usize = 4_096;
pub const MAX_WORDS: usize = 50_000;
pub const MAX_INPUT_BYTES: usize = 96 * 1024;
pub const MAX_WIRE: u64 = 131_072;
pub const OPENAI_SETTINGS: &str = "emphasis-openai-v1;responses;strict;store=false;max_output=8192";
pub const ANTHROPIC_SETTINGS: &str = "emphasis-anthropic-v1;messages;json_schema;max_output=8192";

/// Typed reason an emphasis input or model result is refused before use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EmphasisRefusal {
    #[error("no cue has verified word timing")]
    EmptyTranscript,
    #[error("transcript language is unsupported")]
    UnsupportedLanguage,
    #[error("emphasis cues or word positions are invalid")]
    InvalidSegments,
    #[error("emphasis payload exceeds its limit")]
    Payload,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmphasisSettings {
    /// Source language shared by every cue.
    pub language: LanguageTag,
    /// Most words the model may choose per cue.
    pub max_per_cue: usize,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmphasisWord {
    /// Position of the word in the cue's verified word records.
    pub position: u32,
    pub text: NfcText,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmphasisSegment {
    /// Stable `<caption item>:<cue>` identity; never a timestamp.
    pub id: NfcText,
    pub text: NfcText,
    pub words: Vec<EmphasisWord>,
}
/// Ordered cues with their verified words and the per-cue limit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmphasisInput {
    pub segments: Vec<EmphasisSegment>,
    pub settings: EmphasisSettings,
}
impl EmphasisInput {
    /// Refuse empty, unsupported or oversized input before consent.
    pub fn validate(&self) -> Result<(), EmphasisRefusal> {
        if !crate::supports_text_language(&self.settings.language) {
            return Err(EmphasisRefusal::UnsupportedLanguage);
        }
        if self.segments.is_empty() {
            return Err(EmphasisRefusal::EmptyTranscript);
        }
        if self.segments.len() > MAX_SEGMENTS {
            return Err(EmphasisRefusal::Payload);
        }
        if !(1..=captions::MAX_EMPHASIS_WORDS).contains(&self.settings.max_per_cue) {
            return Err(EmphasisRefusal::InvalidSegments);
        }
        let mut ids = BTreeSet::new();
        let mut words = 0usize;
        let mut bytes = 0usize;
        for segment in &self.segments {
            if segment.id.as_str().trim().is_empty()
                || segment.id.as_str().len() > 256
                || segment.text.as_str().trim().is_empty()
                || segment.words.is_empty()
                || !ids.insert(&segment.id)
            {
                return Err(EmphasisRefusal::InvalidSegments);
            }
            bytes = bytes
                .saturating_add(segment.id.as_str().len())
                .saturating_add(segment.text.as_str().len());
            let mut positions = BTreeSet::new();
            for word in &segment.words {
                if word.text.as_str().trim().is_empty()
                    || word.text.as_str().len() > 256
                    || !positions.insert(word.position)
                {
                    return Err(EmphasisRefusal::InvalidSegments);
                }
                bytes = bytes.saturating_add(word.text.as_str().len() + 8);
            }
            words += segment.words.len();
        }
        if words > MAX_WORDS || bytes > MAX_INPUT_BYTES {
            return Err(EmphasisRefusal::Payload);
        }
        Ok(())
    }
    /// Exact complete input bound to this invocation's consent.
    pub fn consent_text(&self) -> Result<NfcText, Error> {
        self.validate()?;
        let text = serde_json::to_string(self).map_err(|_| Error::Payload)?;
        if text.len() > MAX_INPUT_BYTES {
            return Err(EmphasisRefusal::Payload.into());
        }
        Ok(text.into())
    }
    /// Resolve the proposal against this exact input: every cue must have been sent, positions
    /// must be words of that cue, unique and within the per-cue limit. Anything else refuses
    /// the whole result; nothing is clamped or dropped silently. Positions come back ascending.
    pub fn resolve(
        &self,
        proposal: EmphasisProposal,
    ) -> Result<Vec<EmphasisSelection>, EmphasisRefusal> {
        self.validate()?;
        let mut seen = BTreeSet::new();
        let mut selections = Vec::with_capacity(proposal.selections.len());
        for mut selection in proposal.selections {
            let segment = self
                .segments
                .iter()
                .find(|segment| segment.id == selection.segment)
                .ok_or(EmphasisRefusal::InvalidSegments)?;
            let valid: BTreeSet<_> = segment.words.iter().map(|word| word.position).collect();
            selection.positions.sort_unstable();
            if !seen.insert(selection.segment.clone())
                || selection.positions.len() > self.settings.max_per_cue
                || selection
                    .positions
                    .windows(2)
                    .any(|pair| matches!(pair, [a, b] if a == b))
                || selection
                    .positions
                    .iter()
                    .any(|position| !valid.contains(position))
            {
                return Err(EmphasisRefusal::InvalidSegments);
            }
            selections.push(selection);
        }
        Ok(selections)
    }
}

/// The words the model chose inside one cue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmphasisSelection {
    pub segment: NfcText,
    pub positions: Vec<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmphasisProposal {
    pub selections: Vec<EmphasisSelection>,
}

/// Blocking single-attempt provider seam; adapters never fall back or retry.
pub trait EmphasisSelector: Send + Sync {
    fn config(&self) -> &ProviderConfig;
    fn capabilities(&self, scope: &Scope) -> Capabilities;
    fn estimate(&self, scope: &Scope) -> CostEstimate;
    /// Provider-neutral upload disclosure for this exact scope; a local adapter
    /// returns `Ok(None)`.
    fn disclosure(&self, _scope: &Scope) -> Result<Option<crate::UploadDisclosure>, Error> {
        Ok(None)
    }
    fn select(
        &self,
        input: &EmphasisInput,
        request: &Request<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<EmphasisProposal, Error>;
}
pub(super) fn prepare(
    provider: &dyn EmphasisSelector,
    input: &EmphasisInput,
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
        Operation::SelectEmphasis,
    )
}
/// Validate consent and cost, execute one provider invocation and resolve its positions.
pub fn select_emphasis(
    provider: &dyn EmphasisSelector,
    input: &EmphasisInput,
    request: &Request<'_>,
    hooks: &dyn ExecutionHooks,
) -> Result<Vec<EmphasisSelection>, Error> {
    crate::contracts::checkpoint(hooks, Stage::SelectEmphasis)?;
    prepare(provider, input, request)?;
    let proposal = provider.select(input, request, hooks)?;
    crate::contracts::checkpoint(hooks, Stage::SelectEmphasis)?;
    Ok(input.resolve(proposal)?)
}
