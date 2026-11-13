//! The persisted captions choices: languages, providers and models.
use crate::{Error, Result};
use captions::LanguageTag;
use std::collections::BTreeMap;

/// Persisted explicit choices. Consent, glossary and private text are never preferences.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    /// `auto` or one of the spoken languages this app's ASR supports.
    pub source: String,
    pub target: String,
    pub transcription: String,
    /// Downloaded local weight choice; remote ids persist in AiPreferences.
    #[serde(rename = "transcription_model")]
    pub local_transcription_model: String,
    /// Transient workflow snapshot; remote choices persist only in AiPreferences.
    #[serde(skip)]
    pub transcription_model: String,
    /// Text-task provider shared by translation and filler classification. Each task's
    /// model is its own `AiPreferences` row for this provider.
    pub text_provider: String,
    /// Transient snapshot of the Translate row for `text_provider`.
    #[serde(skip)]
    pub translation_model: String,
    /// Transient snapshot of the ClassifyFillers row for `text_provider`.
    #[serde(skip)]
    pub fillers_model: String,
    pub bilingual: bool,
    /// Last spoken-language choice per project; absent means Auto.
    pub project_sources: BTreeMap<project::ProjectId, String>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            source: "auto".into(),
            target: "en".into(),
            transcription: "whisper.cpp".into(),
            local_transcription_model: "whisper-large-v3-turbo".into(),
            transcription_model: "whisper-large-v3-turbo".into(),
            text_provider: "openai".into(),
            translation_model: ai::defaults::model("openai", ai::Operation::Translate).into(),
            fillers_model: ai::defaults::model("openai", ai::Operation::ClassifyFillers).into(),
            bilingual: false,
            project_sources: BTreeMap::new(),
        }
    }
}
/// Every spoken language the configured ASR adapters actually accept.
pub const SOURCE_LANGUAGES: [&str; 3] = ["vi", "en", "zh"];
impl Options {
    pub fn validate(&self) -> Result<()> {
        if (self.source != "auto" && !SOURCE_LANGUAGES.contains(&self.source.as_str()))
            || !["vi", "en", "zh-Hans", "zh-Hant"].contains(&self.target.as_str())
        {
            return Err(Error::Invalid("subtitle language"));
        }
        if self.project_sources.len() > 1024
            || self
                .project_sources
                .values()
                .any(|source| source != "auto" && !SOURCE_LANGUAGES.contains(&source.as_str()))
        {
            return Err(Error::Invalid("subtitle project language"));
        }
        engine::providers::choose_transcription(
            &self.transcription,
            (!self.transcription_model.is_empty()).then_some(self.transcription_model.as_str()),
        )?;
        engine::providers::choose_translation(
            &self.text_provider,
            (!self.translation_model.is_empty()).then_some(self.translation_model.as_str()),
        )?;
        engine::providers::choose_filler(
            &self.text_provider,
            (!self.fillers_model.is_empty()).then_some(self.fillers_model.as_str()),
        )?;
        Ok(())
    }
    /// Auto is the default until a project stores an explicit choice.
    pub fn source_for(&self, project: project::ProjectId) -> &str {
        self.project_sources
            .get(&project)
            .map_or("auto", String::as_str)
    }
    pub fn load_source(&mut self, project: project::ProjectId) {
        self.source = self.source_for(project).to_owned();
    }
    pub fn remember_source(&mut self, project: project::ProjectId) {
        if self.source_for(project) != self.source {
            self.project_sources.insert(project, self.source.clone());
        }
    }
    /// Provider filtering sees Auto as "no language filter", never as a tag.
    pub(crate) fn source_tag(&self) -> Option<LanguageTag> {
        (self.source != "auto")
            .then(|| LanguageTag::parse(&self.source).ok())
            .flatten()
    }
    pub(crate) fn transcription_supported(&self) -> bool {
        self.validate().is_ok()
            && engine::providers::provider_options_for(
                engine::providers::CatalogOperation::Transcribe,
                self.source_tag().as_ref(),
            )
            .iter()
            .any(|p| {
                p.id == self.transcription
                    && (!p.local || p.models.contains(&self.transcription_model))
            })
    }
}
