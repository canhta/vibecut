//! Lists the provider and model catalog the user can select from.
use crate::EditError;
use captions::LanguageTag;
use std::path::{Path, PathBuf};

/// One selectable transcription provider with its explicit model identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranscriptionProvider {
    Whisper { model: String },
    Gipformer { model: String },
    OpenAi { model: String },
    ElevenLabs { model: String },
}
/// One selectable translation provider with its explicit model identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranslationProvider {
    OpenAi { model: String },
    Anthropic { model: String },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogOperation {
    Transcribe,
    Translate,
    /// Transcript-segment upload for clip suggestions; no audio crosses the wire.
    SuggestClips,
}
/// Offline capability data used by CLI and UI pickers. No provider is constructed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderOption {
    pub id: &'static str,
    pub label: &'static str,
    pub operation: CatalogOperation,
    pub local: bool,
    pub models: Vec<String>,
    pub languages: Vec<String>,
    pub target_languages: Vec<String>,
    pub native_timing: bool,
}
/// Public model catalog data: identity, licence, source and verified file sizes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelFileInfo {
    pub name: String,
    pub source: String,
    pub sha256: String,
    pub size: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: String,
    pub license: String,
    pub license_url: String,
    pub source: String,
    pub languages: Vec<String>,
    pub files: Vec<ModelFileInfo>,
    pub total_bytes: u64,
}
impl TranscriptionProvider {
    pub fn id(&self) -> &'static str {
        match self {
            Self::Whisper { .. } => "whisper.cpp",
            Self::Gipformer { .. } => "gipformer",
            Self::OpenAi { .. } => "openai",
            Self::ElevenLabs { .. } => "elevenlabs",
        }
    }
    pub fn model(&self) -> &str {
        match self {
            Self::Whisper { model }
            | Self::Gipformer { model }
            | Self::OpenAi { model }
            | Self::ElevenLabs { model } => model,
        }
    }
    pub fn is_remote(&self) -> bool {
        matches!(self, Self::OpenAi { .. } | Self::ElevenLabs { .. })
    }
    pub fn has_native_timing(&self) -> bool {
        matches!(self, Self::Whisper { .. } | Self::ElevenLabs { .. })
    }
}
impl Default for TranscriptionProvider {
    fn default() -> Self {
        Self::Whisper {
            model: "whisper-large-v3-turbo".into(),
        }
    }
}
impl TranslationProvider {
    pub fn id(&self) -> &'static str {
        match self {
            Self::OpenAi { .. } => "openai",
            Self::Anthropic { .. } => "anthropic",
        }
    }
    pub fn model(&self) -> &str {
        match self {
            Self::OpenAi { model } | Self::Anthropic { model } => model,
        }
    }
}
pub fn provider_options() -> Vec<ProviderOption> {
    let tags = |values: &[&str]| values.iter().map(|v| (*v).to_owned()).collect();
    vec![
        ProviderOption {
            id: "whisper.cpp",
            label: "whisper.cpp (local)",
            operation: CatalogOperation::Transcribe,
            local: true,
            models: vec!["whisper-large-v3-turbo".into()],
            languages: tags(&["vi", "en", "zh"]),
            target_languages: Vec::new(),
            native_timing: true,
        },
        ProviderOption {
            id: "gipformer",
            label: "Gipformer (local, Vietnamese)",
            operation: CatalogOperation::Transcribe,
            local: true,
            models: vec![ai::gipformer::MODEL_ID.into()],
            languages: tags(&["vi"]),
            target_languages: Vec::new(),
            native_timing: false,
        },
        ProviderOption {
            id: "openai",
            label: "OpenAI",
            operation: CatalogOperation::Transcribe,
            local: false,
            models: vec![ai::openai::MODEL.into()],
            languages: tags(&ai::openai::LANGUAGES),
            target_languages: Vec::new(),
            native_timing: false,
        },
        ProviderOption {
            id: "elevenlabs",
            label: "ElevenLabs Scribe",
            operation: CatalogOperation::Transcribe,
            local: false,
            models: vec![ai::elevenlabs::MODEL.into()],
            languages: tags(&ai::elevenlabs::LANGUAGES),
            target_languages: Vec::new(),
            native_timing: true,
        },
        ProviderOption {
            id: "openai",
            label: "OpenAI (translation)",
            operation: CatalogOperation::Translate,
            local: false,
            models: vec![ai::openai::TRANSLATION_MODEL.into()],
            languages: tags(&ai::openai::TRANSLATION_LANGUAGES),
            target_languages: tags(&ai::openai::TRANSLATION_TARGET_LANGUAGES),
            native_timing: false,
        },
        ProviderOption {
            id: "anthropic",
            label: "Anthropic (translation)",
            operation: CatalogOperation::Translate,
            local: false,
            models: vec![ai::anthropic::MODEL.into()],
            languages: tags(&ai::anthropic::TRANSLATION_LANGUAGES),
            target_languages: tags(&ai::anthropic::TRANSLATION_TARGET_LANGUAGES),
            native_timing: false,
        },
    ]
}
pub fn provider_options_for(
    operation: CatalogOperation,
    language: Option<&LanguageTag>,
) -> Vec<ProviderOption> {
    provider_options()
        .into_iter()
        .filter(|option| option.operation == operation)
        .filter(|option| {
            language.is_none_or(|tag| option.languages.iter().any(|value| value == tag.as_str()))
        })
        .collect()
}
pub fn parse_language(value: &str) -> Result<LanguageTag, EditError> {
    Ok(LanguageTag::parse(value)?)
}
pub fn normalize_target_tag(target: &LanguageTag) -> Result<LanguageTag, EditError> {
    normalize_target(target)
}
pub fn choose_transcription(
    provider: &str,
    model: Option<&str>,
) -> Result<TranscriptionProvider, EditError> {
    let option = provider_options()
        .into_iter()
        .find(|option| {
            option.operation == CatalogOperation::Transcribe
                && (option.id == provider || (provider == "whisper" && option.id == "whisper.cpp"))
        })
        .ok_or(EditError::Unsupported("unknown transcription provider"))?;
    let model = model.unwrap_or_else(|| option.models.first().map_or("", String::as_str));
    if !option.models.iter().any(|value| value == model) {
        return Err(EditError::Unsupported("unsupported model for provider"));
    }
    match option.id {
        "whisper.cpp" => Ok(TranscriptionProvider::Whisper {
            model: model.into(),
        }),
        "gipformer" => Ok(TranscriptionProvider::Gipformer {
            model: model.into(),
        }),
        "openai" => Ok(TranscriptionProvider::OpenAi {
            model: model.into(),
        }),
        "elevenlabs" => Ok(TranscriptionProvider::ElevenLabs {
            model: model.into(),
        }),
        _ => Err(EditError::Unsupported("unknown transcription provider")),
    }
}
pub fn choose_translation(
    provider: &str,
    model: Option<&str>,
) -> Result<TranslationProvider, EditError> {
    let option = provider_options()
        .into_iter()
        .find(|option| option.operation == CatalogOperation::Translate && option.id == provider)
        .ok_or(EditError::Unsupported("unknown translation provider"))?;
    let model = model.unwrap_or_else(|| option.models.first().map_or("", String::as_str));
    if !option.models.iter().any(|value| value == model) {
        return Err(EditError::Unsupported("unsupported model for provider"));
    }
    match option.id {
        "openai" => Ok(TranslationProvider::OpenAi {
            model: model.into(),
        }),
        "anthropic" => Ok(TranslationProvider::Anthropic {
            model: model.into(),
        }),
        _ => Err(EditError::Unsupported("unknown translation provider")),
    }
}
impl ModelInfo {
    pub(crate) fn from_model(model: &ai::models::Model) -> Result<Self, EditError> {
        let total_bytes = model
            .files
            .iter()
            .try_fold(0u64, |sum, file| sum.checked_add(file.size))
            .ok_or(EditError::Limit)?;
        Ok(Self {
            id: model.id.clone(),
            license: model.license.clone(),
            license_url: model.license_url.clone(),
            source: model.source.clone(),
            languages: model
                .languages
                .iter()
                .map(|value| value.as_str().to_owned())
                .collect(),
            files: model
                .files
                .iter()
                .map(|file| ModelFileInfo {
                    name: file.name.clone(),
                    source: file.url.clone(),
                    sha256: file.sha256.clone(),
                    size: file.size,
                })
                .collect(),
            total_bytes,
        })
    }
}
pub fn model_infos() -> Result<Vec<ModelInfo>, EditError> {
    ai::models::catalog()?
        .iter()
        .map(ModelInfo::from_model)
        .collect()
}
pub fn model_info(id: &str) -> Result<ModelInfo, EditError> {
    let model = catalog_model(id)?;
    ModelInfo::from_model(&model)
}
/// The catalog model with this id, as the model manager and the CLI download it.
pub fn catalog_model(id: &str) -> Result<ai::models::Model, EditError> {
    ai::models::catalog()?
        .into_iter()
        .find(|model| model.id == id)
        .ok_or(EditError::Unsupported("unknown model"))
}
/// Default location shared by model bundles and ASR checkpoints.
pub fn default_cache_root() -> Result<PathBuf, EditError> {
    Ok(ai::models::cache_dir()?)
}
/// Find the explicitly configured or bundled local ASR helper without launching
/// it or downloading weights. Shared by desktop and CLI composition roots.
pub fn local_asr_runner() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("VIBECUT_LOCAL_ASR") {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(parent) = executable.parent()
    {
        let path = parent.join("vibecut-local-asr");
        if path.is_file() {
            return Some(path);
        }
    }
    let path = Path::new("target")
        .join("release")
        .join("vibecut-local-asr");
    path.is_file().then_some(path)
}
pub(crate) fn normalize_target(target: &LanguageTag) -> Result<LanguageTag, EditError> {
    if target.as_str() == "zh" {
        Ok(LanguageTag::parse("zh-Hans")?)
    } else {
        Ok(target.clone())
    }
}
