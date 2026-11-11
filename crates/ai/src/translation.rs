//! Contextual translation contract. Texts, IDs, glossary and style are consent-bound.
use crate::{
    Capabilities, CostEstimate, Error, ExecutionHooks, Operation, ProviderConfig, Request,
    SampleRange, Scope, Stage,
};
use captions::{LanguageTag, NfcText};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_WINDOW_BYTES: usize = 32 * 1024;
/// Wire-payload floor of a translation request: the structured-output envelope
/// around a full window.
const MAX_JSON_BYTES: u64 = 64 * 1024;
pub const MAX_SEGMENTS: usize = 64;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranslationSegment {
    pub id: NfcText,
    pub text: NfcText,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlossaryTerm {
    pub term: NfcText,
    /// None means preserve the term verbatim (names/do-not-translate).
    pub translation: Option<NfcText>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranslationWindow {
    pub source_language: LanguageTag,
    pub target_language: LanguageTag,
    pub segments: Vec<TranslationSegment>,
    pub before: Vec<NfcText>,
    pub after: Vec<NfcText>,
    pub glossary: Vec<GlossaryTerm>,
    pub style: NfcText,
}
impl TranslationWindow {
    /// Canonical complete request text for Scope::texts. No caller-provided
    /// fingerprint substitutes for the exact payload's in-memory consent binding.
    pub fn consent_text(&self) -> Result<NfcText, Error> {
        self.validate()?;
        let text = serde_json::to_string(self).map_err(|_| Error::Invalid("translation window"))?;
        if text.len() > MAX_WINDOW_BYTES {
            return Err(Error::Payload);
        }
        Ok(text.into())
    }
    pub fn validate(&self) -> Result<(), Error> {
        if self.segments.is_empty()
            || self.segments.len() > MAX_SEGMENTS
            || self.before.len() > 16
            || self.after.len() > 16
            || self.glossary.len() > 128
        {
            return Err(Error::Payload);
        }
        let mut ids = BTreeSet::new();
        let mut bytes = self.style.as_str().len();
        for s in &self.segments {
            if s.id.as_str().trim().is_empty()
                || s.id.as_str().len() > 256
                || s.text.as_str().trim().is_empty()
                || !ids.insert(&s.id)
            {
                return Err(Error::Invalid("translation segment"));
            }
            bytes = bytes
                .saturating_add(s.id.as_str().len())
                .saturating_add(s.text.as_str().len());
        }
        let mut terms = BTreeSet::new();
        for g in &self.glossary {
            if g.term.as_str().trim().is_empty()
                || !terms.insert(&g.term)
                || g.translation
                    .as_ref()
                    .is_some_and(|t| t.as_str().trim().is_empty())
            {
                return Err(Error::Invalid("glossary term"));
            }
            bytes = bytes
                .saturating_add(g.term.as_str().len())
                .saturating_add(g.translation.as_ref().map_or(0, |t| t.as_str().len()));
        }
        for t in self.before.iter().chain(&self.after) {
            bytes = bytes.saturating_add(t.as_str().len());
        }
        if bytes > MAX_WINDOW_BYTES {
            return Err(Error::Payload);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Translation {
    pub source_language: LanguageTag,
    pub target_language: LanguageTag,
    pub texts: BTreeMap<NfcText, NfcText>,
    pub number_warnings: Vec<NfcText>,
}
impl Scope {
    /// Consent scope of one translation window. The fingerprint identifies the
    /// exact payload for the caller's cache and consent records.
    pub fn translation(
        provider: ProviderConfig,
        mode: NfcText,
        window: &TranslationWindow,
        payload_fingerprint: NfcText,
    ) -> Result<Self, Error> {
        let payload = window.consent_text()?;
        let bytes = u64::try_from(payload.as_str().len()).map_err(|_| Error::Payload)?;
        Ok(Self {
            provider,
            operation: Operation::Translate,
            payload_fingerprint,
            mode,
            texts: vec![payload],
            source_language: window.source_language.clone(),
            target_language: Some(window.target_language.clone()),
            source_samples: SampleRange::new(0, 1)?,
            sample_rate: 1,
            max_bytes: bytes.max(MAX_JSON_BYTES),
        })
    }
}
pub trait Translator: Send + Sync {
    fn config(&self) -> &ProviderConfig;
    fn capabilities(&self, scope: &Scope) -> Capabilities;
    fn estimate(&self, scope: &Scope) -> CostEstimate;
    /// Provider-neutral upload disclosure for this exact scope; local adapters
    /// return `Ok(None)` and never upload.
    fn disclosure(&self, _scope: &Scope) -> Result<Option<crate::UploadDisclosure>, Error> {
        Ok(None)
    }
    /// Blocking, single attempt. Engine owns all schema/network retry and cost budgets.
    fn translate(
        &self,
        window: &TranslationWindow,
        request: &Request<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Translation, Error>;
}
pub fn validate_translation(
    window: &TranslationWindow,
    result: &Translation,
) -> Result<Vec<NfcText>, Error> {
    window.validate()?;
    if result.source_language != window.source_language
        || result.target_language != window.target_language
        || result.texts.len() != window.segments.len()
    {
        return Err(Error::Schema);
    }
    let mut warnings = Vec::new();
    if result
        .texts
        .values()
        .try_fold(0usize, |sum, t| sum.checked_add(t.as_str().len()))
        .is_none_or(|n| n > MAX_WINDOW_BYTES * 2)
    {
        return Err(Error::Schema);
    }
    for s in &window.segments {
        let translated = result.texts.get(&s.id).ok_or(Error::Schema)?;
        if translated.as_str().trim().is_empty() || translated.as_str().len() > MAX_WINDOW_BYTES {
            return Err(Error::Schema);
        }
        let digits = |text: &str| {
            text.chars()
                .filter(char::is_ascii_digit)
                .collect::<BTreeSet<_>>()
        };
        if digits(s.text.as_str()) != digits(translated.as_str()) {
            warnings.push(s.id.clone());
        }
        for g in &window.glossary {
            if s.text.as_str().contains(g.term.as_str())
                && !translated
                    .as_str()
                    .contains(g.translation.as_ref().unwrap_or(&g.term).as_str())
            {
                return Err(Error::Schema);
            }
        }
    }
    Ok(warnings)
}
/// Shared checks also apply to non-OpenAI adapters. No source timing is assigned
/// to translated words. Warnings are recomputed, never trusted from a provider.
pub fn translate(
    provider: &dyn Translator,
    window: &TranslationWindow,
    request: &Request<'_>,
    hooks: &dyn ExecutionHooks,
) -> Result<Translation, Error> {
    crate::contracts::checkpoint(hooks, Stage::Translate)?;
    prepare(provider, window, request)?;
    let mut result = provider.translate(window, request, hooks)?;
    crate::contracts::checkpoint(hooks, Stage::Translate)?;
    result.number_warnings = validate_translation(window, &result)?;
    Ok(result)
}
pub(crate) fn prepare(
    provider: &dyn Translator,
    window: &TranslationWindow,
    request: &Request<'_>,
) -> Result<(), Error> {
    if provider.config() != &request.scope.provider {
        return Err(Error::Unsupported);
    }
    if request.scope.source_language != window.source_language
        || request.scope.target_language.as_ref() != Some(&window.target_language)
    {
        return Err(Error::ConsentRequired);
    }
    let text = window.consent_text()?;
    if request.scope.texts != [text.clone()] {
        return Err(Error::ConsentRequired);
    }
    if text.as_str().len() as u64 > request.scope.max_bytes {
        return Err(Error::Payload);
    }
    request.prepare(
        &provider.capabilities(request.scope),
        &provider.estimate(request.scope),
        Operation::Translate,
    )
}
