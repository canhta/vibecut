//! Chapter, title and description generation from ordered transcript segments.
mod remote;
use crate::{
    Capabilities, CostEstimate, Error, ExecutionHooks, Operation, ProviderConfig, Request, Scope,
    Stage,
};
use captions::{LanguageTag, NfcText};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use time::{TICKS_PER_SECOND, Tick};

pub use remote::{AnthropicChapterGenerator, OpenAiChapterGenerator};

pub const MODE: &str = "transcript-chapters";
impl Scope {
    /// Consent scope of one chapter request, bound to the complete input text.
    pub fn chapters(
        provider: ProviderConfig,
        input: &ChapterInput,
        payload_fingerprint: NfcText,
    ) -> Result<Self, Error> {
        Self::text(
            provider,
            Operation::GenerateChapters,
            MODE,
            input.consent_text()?,
            payload_fingerprint,
            input.settings.language.clone(),
            MAX_WIRE,
        )
    }
}
/// The versioned chapter-document schema written to `.json` sidecars.
pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_SEGMENTS: usize = 10_000;
pub const MAX_CHAPTERS: usize = 50;
pub const MAX_TRANSLATIONS: usize = 4;
pub const MAX_INPUT_BYTES: usize = 96 * 1024;
pub const MAX_TITLE_BYTES: usize = 2_048;
pub const MAX_DESCRIPTION_BYTES: usize = 16 * 1024;
pub const MAX_WIRE: u64 = 131_072;
pub const OPENAI_SETTINGS: &str = "chapters-openai-v1;responses;strict;store=false;max_output=8192";
pub const ANTHROPIC_SETTINGS: &str = "chapters-anthropic-v1;messages;json_schema;max_output=8192";

/// Typed reason a transcript or chapter result is refused before publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ChapterRefusal {
    #[error("transcript has no segments")]
    EmptyTranscript,
    #[error("transcript language is unsupported")]
    UnsupportedLanguage,
    #[error("chapter settings are invalid")]
    InvalidSettings,
    #[error("chapter transcript segments are invalid")]
    InvalidSegments,
    #[error("chapter payload exceeds its limit")]
    Payload,
    #[error("chapter document schema version is unsupported")]
    SchemaVersion,
    #[error("chapter document is invalid")]
    InvalidDocument,
    #[error("chapter translation is invalid")]
    InvalidTranslation,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChapterSegment {
    pub id: NfcText,
    pub text: NfcText,
    pub range: time::SourceRange,
}
impl From<&captions::Segment> for ChapterSegment {
    fn from(segment: &captions::Segment) -> Self {
        Self {
            id: segment.id.clone(),
            text: segment.text.clone(),
            range: segment.range,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChapterSettings {
    /// Source language shared by the transcript and all generated metadata.
    pub language: LanguageTag,
    /// Requested maximum chapters, 1..=MAX_CHAPTERS.
    pub max_chapters: usize,
    /// Positive timeline duration in ticks; every chapter start stays before it.
    pub duration: Tick,
}

/// Ordered, nonoverlapping transcript segments and the requested chapter bounds.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChapterInput {
    pub segments: Vec<ChapterSegment>,
    pub settings: ChapterSettings,
}
impl ChapterInput {
    /// Refuses empty, unsupported, unordered or oversized transcripts before consent.
    pub fn validate(&self) -> Result<(), ChapterRefusal> {
        let settings = &self.settings;
        if !crate::supports_text_language(&settings.language) {
            return Err(ChapterRefusal::UnsupportedLanguage);
        }
        if settings.duration <= Tick::ZERO || !(1..=MAX_CHAPTERS).contains(&settings.max_chapters) {
            return Err(ChapterRefusal::InvalidSettings);
        }
        if self.segments.is_empty() {
            return Err(ChapterRefusal::EmptyTranscript);
        }
        if self.segments.len() > MAX_SEGMENTS {
            return Err(ChapterRefusal::Payload);
        }
        let mut ids = BTreeSet::new();
        let mut end = Tick::ZERO;
        let mut bytes = 0usize;
        for segment in &self.segments {
            if segment.id.as_str().trim().is_empty()
                || segment.id.as_str().len() > 256
                || segment.text.as_str().trim().is_empty()
                || segment.range.start() < end
                || segment.range.end() > settings.duration
                || !ids.insert(&segment.id)
            {
                return Err(ChapterRefusal::InvalidSegments);
            }
            end = segment.range.end();
            bytes = bytes
                .saturating_add(segment.text.as_str().len())
                .saturating_add(segment.id.as_str().len());
        }
        if bytes > MAX_INPUT_BYTES {
            return Err(ChapterRefusal::Payload);
        }
        Ok(())
    }
    /// Exact complete input bound to this invocation's consent.
    pub fn consent_text(&self) -> Result<NfcText, Error> {
        self.validate()?;
        let text = serde_json::to_string(self).map_err(|_| Error::Payload)?;
        if text.len() > MAX_INPUT_BYTES {
            return Err(ChapterRefusal::Payload.into());
        }
        Ok(text.into())
    }
    /// Resolve provider-selected segment IDs to cue-boundary chapter starts.
    pub fn resolve(&self, proposal: ChapterProposal) -> Result<ChapterDocument, Error> {
        self.validate()?;
        if proposal.chapters.is_empty() || proposal.chapters.len() > self.settings.max_chapters {
            return Err(Error::Schema);
        }
        if !valid_title(&proposal.title) || !valid_description(&proposal.description) {
            return Err(Error::Schema);
        }
        let ids: BTreeMap<_, _> = self
            .segments
            .iter()
            .map(|segment| (&segment.id, segment))
            .collect();
        let mut chapters = Vec::with_capacity(proposal.chapters.len());
        let mut previous = None;
        for entry in proposal.chapters {
            if entry.start_segment_id.as_str().is_empty()
                || entry.start_segment_id.as_str().len() > 256
                || !valid_title(&entry.title)
            {
                return Err(Error::Schema);
            }
            let segment = ids.get(&entry.start_segment_id).ok_or(Error::Schema)?;
            let start = segment.range.start();
            if previous.is_some_and(|previous| start <= previous) || start >= self.settings.duration
            {
                return Err(Error::Schema);
            }
            previous = Some(start);
            chapters.push(Chapter {
                start,
                segment_id: entry.start_segment_id,
                title: entry.title,
            });
        }
        ChapterDocument::new(
            self.settings.language.clone(),
            self.settings.duration,
            proposal.title,
            proposal.description,
            chapters,
            Vec::new(),
        )
        .map_err(Into::into)
    }
}

/// One chapter start, snapped to the selected transcript segment's cue boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Chapter {
    pub start: Tick,
    pub segment_id: NfcText,
    pub title: NfcText,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChapterTranslation {
    pub language: LanguageTag,
    pub title: NfcText,
    pub description: NfcText,
    /// One translated title per source chapter, in source order.
    pub chapters: Vec<NfcText>,
}
/// The wire shape of a validated chapter document; `ChapterDocument` admits it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChapterDocumentData {
    schema_version: u32,
    language: LanguageTag,
    duration: Tick,
    title: NfcText,
    description: NfcText,
    chapters: Vec<Chapter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    translations: Vec<ChapterTranslation>,
}
/// Versioned chapter sidecar document; construction and deserialization validate it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "ChapterDocumentData", into = "ChapterDocumentData")]
#[schemars(with = "ChapterDocumentData")]
pub struct ChapterDocument(ChapterDocumentData);
impl ChapterDocument {
    pub fn new(
        language: LanguageTag,
        duration: Tick,
        title: NfcText,
        description: NfcText,
        chapters: Vec<Chapter>,
        translations: Vec<ChapterTranslation>,
    ) -> Result<Self, ChapterRefusal> {
        ChapterDocumentData {
            schema_version: SCHEMA_VERSION,
            language,
            duration,
            title,
            description,
            chapters,
            translations,
        }
        .try_into()
    }
    pub fn schema_version(&self) -> u32 {
        self.0.schema_version
    }
    pub fn language(&self) -> &LanguageTag {
        &self.0.language
    }
    pub fn duration(&self) -> Tick {
        self.0.duration
    }
    pub fn title(&self) -> &NfcText {
        &self.0.title
    }
    pub fn description(&self) -> &NfcText {
        &self.0.description
    }
    pub fn chapters(&self) -> &[Chapter] {
        &self.0.chapters
    }
    pub fn translations(&self) -> &[ChapterTranslation] {
        &self.0.translations
    }
    pub fn validate(&self) -> Result<(), ChapterRefusal> {
        self.0.validate()
    }
    /// Canonical pretty JSON sidecar bytes; validation runs before serialization.
    pub fn to_json(&self) -> Result<Vec<u8>, Error> {
        self.validate()?;
        serde_json::to_vec_pretty(self).map_err(|_| Error::Payload)
    }
    pub fn from_json(bytes: &[u8]) -> Result<Self, Error> {
        let data: ChapterDocumentData = serde_json::from_slice(bytes).map_err(|_| Error::Schema)?;
        data.try_into().map_err(Error::Chapters)
    }
    /// Human-readable `.txt` sidecar: title, description and timestamped chapters.
    pub fn to_text(&self) -> Result<String, Error> {
        self.validate()?;
        let mut text = format!(
            "{}\n\n{}\n",
            self.0.title.as_str(),
            self.0.description.as_str()
        );
        for chapter in &self.0.chapters {
            text.push_str(&format!(
                "\n{} {}",
                timestamp(chapter.start),
                chapter.title.as_str()
            ));
        }
        for translation in &self.0.translations {
            text.push_str(&format!(
                "\n\n[{}]\n{}\n\n{}",
                translation.language.as_str(),
                translation.title.as_str(),
                translation.description.as_str()
            ));
            for (chapter, title) in self.0.chapters.iter().zip(&translation.chapters) {
                text.push_str(&format!(
                    "\n{} {}",
                    timestamp(chapter.start),
                    title.as_str()
                ));
            }
        }
        Ok(text)
    }
}
impl ChapterDocumentData {
    fn validate(&self) -> Result<(), ChapterRefusal> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ChapterRefusal::SchemaVersion);
        }
        if !crate::supports_text_language(&self.language) {
            return Err(ChapterRefusal::UnsupportedLanguage);
        }
        if self.duration <= Tick::ZERO
            || !valid_title(&self.title)
            || !valid_description(&self.description)
            || self.chapters.is_empty()
            || self.chapters.len() > MAX_CHAPTERS
            || self.translations.len() > MAX_TRANSLATIONS
        {
            return Err(ChapterRefusal::InvalidDocument);
        }
        let mut previous = None;
        for chapter in &self.chapters {
            if chapter.start < Tick::ZERO
                || chapter.start >= self.duration
                || chapter.segment_id.as_str().trim().is_empty()
                || chapter.segment_id.as_str().len() > 256
                || !valid_title(&chapter.title)
                || previous.is_some_and(|previous| chapter.start <= previous)
            {
                return Err(ChapterRefusal::InvalidDocument);
            }
            previous = Some(chapter.start);
        }
        let mut languages = BTreeSet::new();
        for translation in &self.translations {
            if translation.language == self.language
                || !crate::supports_text_language(&translation.language)
                || !languages.insert(&translation.language)
                || !valid_title(&translation.title)
                || !valid_description(&translation.description)
                || translation.chapters.len() != self.chapters.len()
                || !translation.chapters.iter().all(valid_title)
            {
                return Err(ChapterRefusal::InvalidTranslation);
            }
        }
        Ok(())
    }
}
impl TryFrom<ChapterDocumentData> for ChapterDocument {
    type Error = ChapterRefusal;
    fn try_from(data: ChapterDocumentData) -> Result<Self, Self::Error> {
        data.validate()?;
        Ok(Self(data))
    }
}
impl From<ChapterDocument> for ChapterDocumentData {
    fn from(value: ChapterDocument) -> Self {
        value.0
    }
}

/// Blocking single-attempt provider seam. Engine may retry the whole invocation;
/// adapters never switch providers themselves.
pub trait ChapterGenerator: Send + Sync {
    fn config(&self) -> &ProviderConfig;
    fn capabilities(&self, scope: &Scope) -> Capabilities;
    fn estimate(&self, scope: &Scope) -> CostEstimate;
    /// Provider-neutral upload disclosure for this exact scope; a local adapter
    /// returns `Ok(None)`.
    fn disclosure(&self, _scope: &Scope) -> Result<Option<crate::UploadDisclosure>, Error> {
        Ok(None)
    }
    fn propose(
        &self,
        input: &ChapterInput,
        request: &Request<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<ChapterProposal, Error>;
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChapterProposal {
    pub title: NfcText,
    pub description: NfcText,
    pub chapters: Vec<ChapterProposalEntry>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChapterProposalEntry {
    pub start_segment_id: NfcText,
    pub title: NfcText,
}
pub(super) fn prepare(
    provider: &dyn ChapterGenerator,
    input: &ChapterInput,
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
        Operation::GenerateChapters,
    )
}
/// Validate consent and cost, execute one provider invocation and resolve its segments.
pub fn generate_chapters(
    provider: &dyn ChapterGenerator,
    input: &ChapterInput,
    request: &Request<'_>,
    hooks: &dyn ExecutionHooks,
) -> Result<ChapterDocument, Error> {
    crate::contracts::checkpoint(hooks, Stage::GenerateChapters)?;
    prepare(provider, input, request)?;
    let proposal = provider.propose(input, request, hooks)?;
    crate::contracts::checkpoint(hooks, Stage::GenerateChapters)?;
    input.resolve(proposal)
}

/// Translate title, description and chapter titles through the shared translation
/// contract. Chapter starts stay in source timeline coordinates.
pub fn translate_chapters(
    translator: &dyn crate::Translator,
    document: &ChapterDocument,
    target_language: LanguageTag,
    style: &NfcText,
    request: &Request<'_>,
    hooks: &dyn ExecutionHooks,
) -> Result<ChapterDocument, Error> {
    document.validate()?;
    if target_language == *document.language() || !crate::supports_text_language(&target_language) {
        return Err(ChapterRefusal::InvalidTranslation.into());
    }
    let mut window = crate::TranslationWindow {
        source_language: document.language().clone(),
        target_language: target_language.clone(),
        segments: Vec::with_capacity(document.chapters().len() + 2),
        before: Vec::new(),
        after: Vec::new(),
        glossary: Vec::new(),
        style: style.clone(),
    };
    window.segments.push(crate::TranslationSegment {
        id: "title".into(),
        text: document.title().clone(),
    });
    window.segments.push(crate::TranslationSegment {
        id: "description".into(),
        text: document.description().clone(),
    });
    for (index, chapter) in document.chapters().iter().enumerate() {
        window.segments.push(crate::TranslationSegment {
            id: format!("chapter-{index}").into(),
            text: chapter.title.clone(),
        });
    }
    let translation = crate::translate(translator, &window, request, hooks)?;
    let mut texts = translation.texts;
    let title = texts.remove(&NfcText::from("title")).ok_or(Error::Schema)?;
    let description = texts
        .remove(&NfcText::from("description"))
        .ok_or(Error::Schema)?;
    if texts.len() != document.chapters().len() {
        return Err(Error::Schema);
    }
    let mut chapters = Vec::with_capacity(document.chapters().len());
    for index in 0..document.chapters().len() {
        chapters.push(
            texts
                .remove(&NfcText::from(format!("chapter-{index}")))
                .ok_or(Error::Schema)?,
        );
    }
    let translation = ChapterTranslation {
        language: target_language,
        title,
        description,
        chapters,
    };
    let mut translations = document
        .translations()
        .iter()
        .filter(|existing| existing.language != translation.language)
        .cloned()
        .collect::<Vec<_>>();
    translations.push(translation);
    ChapterDocument::new(
        document.language().clone(),
        document.duration(),
        document.title().clone(),
        document.description().clone(),
        document.chapters().to_vec(),
        translations,
    )
    .map_err(Into::into)
}

fn valid_title(text: &NfcText) -> bool {
    !text.as_str().trim().is_empty() && text.as_str().len() <= MAX_TITLE_BYTES
}
fn valid_description(text: &NfcText) -> bool {
    !text.as_str().trim().is_empty() && text.as_str().len() <= MAX_DESCRIPTION_BYTES
}
fn timestamp(tick: Tick) -> String {
    let total = tick.0.max(0) / TICKS_PER_SECOND;
    format!(
        "{:02}:{:02}:{:02}",
        total / 3_600,
        (total / 60) % 60,
        total % 60
    )
}
