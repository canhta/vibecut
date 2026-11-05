//! Subtitle workflow: provider choice, speech windows, ASR checkpoints, translation
//! and the existing caption export/burn paths. UI and CLI adapters call this module;
//! consent, cost, chunk merging and checkpoint state are not reimplemented by them.
use crate::cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs, ModelLease};
use crate::jobs::{
    CancelToken, JobBytes, JobId, JobKind, JobOutput, JobRunner, JobSnapshot, JobSpec, JobState,
    ResourceClass,
};
use crate::models::ModelTransfers;
use crate::{ByteBudget, Command, EditError, NewItem, Outcome, Session};
use ai::transport::{ApiKey, UreqTransport};
use ai::{CostEstimate, ExecutionHooks, Location, Money, Request, SampleRange, Scope};
pub use captions::LanguageTag;
use captions::{
    Cue, CueTrack, LanguageProfile, LanguageRegistry, NfcText, SegmentRules, SegmentedCue,
    Transcript,
};
use media::{SourceAsset, SpeechExtractSettings, SpeechWindow};
use project::{Animations, CaptionParams, ItemParams, Project, ProjectId, TrackId, TrackKind};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};
use time::{FrameRate, Rounding, SourceRange, Tick, TimeRange};

const TRANSLATE_ATTEMPTS: u8 = 3;
const MIN_CAPTION_SECONDS: i64 = 30;
const MAX_CAPTION_SECONDS: i64 = 300;
const MAX_JSON_BYTES: usize = 64 * 1024;
const WORK_BYTES: u64 = 64 * 1024 * 1024;

/// One selectable transcription provider with its explicit model identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranscriptionProvider {
    Whisper { model: String },
    Gipformer { model: String },
    OpenAi { model: String },
    ElevenLabs { model: String },
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

/// One selectable translation provider with its explicit model identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranslationProvider {
    OpenAi { model: String },
    Anthropic { model: String },
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
impl ModelInfo {
    fn from_model(model: &ai::models::Model) -> Result<Self, EditError> {
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
    let model = ai_model(id)?;
    ModelInfo::from_model(&model)
}

fn ai_model(id: &str) -> Result<ai::models::Model, EditError> {
    ai::models::catalog()?
        .into_iter()
        .find(|model| model.id == id)
        .ok_or(EditError::Unsupported("unknown model"))
}

/// One explicit model download. The command line is the affirmative action; the
/// downloader still requires the ai-owned consent token for the exact catalog record.
pub struct ModelDownloader {
    transfers: ModelTransfers,
}
impl ModelDownloader {
    pub fn open(budget: ByteBudget) -> Result<Self, EditError> {
        let (transfers, _) =
            ModelTransfers::open_default(budget, 128, Arc::new(UreqTransport::new()))?;
        Ok(Self { transfers })
    }
    pub fn submit(&self, runner: &mut JobRunner, id: &str) -> Result<JobId, EditError> {
        let model = ai_model(id)?;
        let consent = ai::models::DownloadConsent::grant(&model)?;
        let (job, _cancel) = self.transfers.submit(runner, model, Some(consent))?;
        Ok(job)
    }
}

/// Default location shared by model bundles and ASR checkpoints.
pub fn default_cache_root() -> Result<PathBuf, EditError> {
    Ok(ai::models::cache_dir()?)
}

#[derive(Clone)]
pub struct Credentials {
    openai: Option<ApiKey>,
    elevenlabs: Option<ApiKey>,
    anthropic: Option<ApiKey>,
}
impl Credentials {
    /// Secrets are resolved by the composition root; they never enter logs or errors.
    pub fn new(
        openai: Option<String>,
        elevenlabs: Option<String>,
        anthropic: Option<String>,
    ) -> Result<Self, EditError> {
        let key = |value: Option<String>| value.map(ApiKey::new).transpose();
        Ok(Self {
            openai: key(openai)?,
            elevenlabs: key(elevenlabs)?,
            anthropic: key(anthropic)?,
        })
    }
    pub(crate) fn openai(&self) -> Result<&ApiKey, EditError> {
        self.openai
            .as_ref()
            .ok_or(EditError::CredentialMissing("OPENAI_API_KEY"))
    }
    fn elevenlabs(&self) -> Result<&ApiKey, EditError> {
        self.elevenlabs
            .as_ref()
            .ok_or(EditError::CredentialMissing("ELEVENLABS_API_KEY"))
    }
    pub(crate) fn anthropic(&self) -> Result<&ApiKey, EditError> {
        self.anthropic
            .as_ref()
            .ok_or(EditError::CredentialMissing("ANTHROPIC_API_KEY"))
    }
}

/// Local execution choices. The runner is the shipped Gipformer helper, never a model.
#[derive(Clone, Debug)]
pub struct Runtime {
    pub use_gpu: bool,
    pub threads: u8,
    pub gipformer_runner: Option<PathBuf>,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            use_gpu: cfg!(target_os = "macos"),
            threads: 4,
            gipformer_runner: None,
        }
    }
}

struct IdleHooks;
impl ExecutionHooks for IdleHooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn stage(&self, _: ai::Stage) {}
}

enum TranscriberKind {
    Whisper(Arc<ai::whisper::Whisper>),
    Gipformer(Arc<ai::gipformer::Gipformer>),
    OpenAi(Arc<ai::openai::OpenAiTranscriber>),
    ElevenLabs(Arc<ai::elevenlabs::ElevenLabsTranscriber>),
    Other(Arc<dyn ai::Transcriber>),
}
impl TranscriberKind {
    fn as_transcriber(&self) -> &dyn ai::Transcriber {
        match self {
            Self::Whisper(value) => value.as_ref(),
            Self::Gipformer(value) => value.as_ref(),
            Self::OpenAi(value) => value.as_ref(),
            Self::ElevenLabs(value) => value.as_ref(),
            Self::Other(value) => value.as_ref(),
        }
    }
    fn whisper(&self) -> Option<&ai::whisper::Whisper> {
        match self {
            Self::Whisper(value) => Some(value.as_ref()),
            _ => None,
        }
    }
}

enum TranslatorKind {
    OpenAi(Arc<ai::openai::OpenAiTranslator>),
    Anthropic(Arc<ai::anthropic::AnthropicTranslator>),
    Other(Arc<dyn ai::Translator>),
}
impl TranslatorKind {
    fn as_translator(&self) -> &dyn ai::Translator {
        match self {
            Self::OpenAi(value) => value.as_ref(),
            Self::Anthropic(value) => value.as_ref(),
            Self::Other(value) => value.as_ref(),
        }
    }
}

/// Resolved adapter instances and the leases that keep native model files immutable.
pub struct Providers {
    transcriber: TranscriberKind,
    aligner: Option<Arc<dyn ai::Aligner>>,
    translator: Option<TranslatorKind>,
    transcription: TranscriptionProvider,
    translation: Option<TranslationProvider>,
    transcription_mode: NfcText,
    translation_mode: Option<NfcText>,
    needs_alignment: bool,
    _leases: Vec<ModelLease>,
}
impl Providers {
    pub fn open(
        transcription: TranscriptionProvider,
        translation: Option<TranslationProvider>,
        cache: &mut ArtifactCache,
        credentials: &Credentials,
        runtime: &Runtime,
    ) -> Result<Self, EditError> {
        let mut leases = Vec::new();
        let (transcriber, needs_alignment, mode) = match &transcription {
            TranscriptionProvider::Whisper { model } => {
                let metadata = ai_model(model)?;
                let lease = cache
                    .model(&metadata, &IdleHooks)?
                    .ok_or(EditError::ModelMissing("whisper-large-v3-turbo"))?;
                let path = lease.file(
                    &metadata
                        .files
                        .first()
                        .ok_or(EditError::ModelMissing("whisper-large-v3-turbo"))?
                        .name,
                )?;
                let device = if runtime.use_gpu {
                    ai::whisper::Device::Gpu
                } else {
                    ai::whisper::Device::Cpu
                };
                let value = ai::whisper::Whisper::load(
                    &metadata,
                    &path,
                    device,
                    runtime.threads,
                    &IdleHooks,
                )?;
                leases.push(lease);
                (
                    TranscriberKind::Whisper(Arc::new(value)),
                    false,
                    "transcribe",
                )
            }
            TranscriptionProvider::Gipformer { model } => {
                let metadata = ai_model(model)?;
                let lease = cache
                    .model(&metadata, &IdleHooks)?
                    .ok_or(EditError::ModelMissing("gipformer-68m-rnnt-int8"))?;
                let runner = runtime
                    .gipformer_runner
                    .as_deref()
                    .ok_or(EditError::RunnerMissing)?;
                let value = ai::gipformer::Gipformer::load(
                    &metadata,
                    lease.directory(),
                    runner,
                    ai::gipformer::Device::Cpu,
                    runtime.threads,
                    &IdleHooks,
                )?;
                leases.push(lease);
                (
                    TranscriberKind::Gipformer(Arc::new(value)),
                    true,
                    "transcribe",
                )
            }
            TranscriptionProvider::OpenAi { model } => {
                let config = ai::ProviderConfig::new(
                    "openai".into(),
                    model.as_str().into(),
                    ai::openai::SETTINGS_FINGERPRINT.into(),
                    Location::Remote {
                        backend: ai::openai::BACKEND.into(),
                        base_url: ai::openai::BASE_URL.into(),
                        auth_ref: "OPENAI_API_KEY".into(),
                    },
                )?;
                let value = ai::openai::OpenAiTranscriber::new(
                    config,
                    credentials.openai()?.clone(),
                    Arc::new(UreqTransport::new()),
                )?;
                (TranscriberKind::OpenAi(Arc::new(value)), true, "file")
            }
            TranscriptionProvider::ElevenLabs { model } => {
                let config = ai::ProviderConfig::new(
                    "elevenlabs".into(),
                    model.as_str().into(),
                    ai::elevenlabs::SETTINGS_FINGERPRINT.into(),
                    Location::Remote {
                        backend: ai::elevenlabs::BACKEND.into(),
                        base_url: ai::elevenlabs::BASE_URL.into(),
                        auth_ref: "ELEVENLABS_API_KEY".into(),
                    },
                )?;
                let value = ai::elevenlabs::ElevenLabsTranscriber::new(
                    config,
                    credentials.elevenlabs()?.clone(),
                    Arc::new(UreqTransport::new()),
                )?;
                (TranscriberKind::ElevenLabs(Arc::new(value)), false, "file")
            }
        };
        let aligner = if needs_alignment {
            Some(load_aligner(cache, &mut leases)?)
        } else {
            None
        };
        let (translator, translation_mode) = match &translation {
            None => (None, None),
            Some(value) => {
                let (kind, mode) = build_translator(value, credentials)?;
                (Some(kind), Some(mode))
            }
        };
        Ok(Self {
            transcriber,
            aligner,
            translator,
            transcription,
            translation,
            transcription_mode: mode.into(),
            translation_mode,
            needs_alignment,
            _leases: leases,
        })
    }

    /// Attach a translator after an operation has already resolved its source
    /// language. This keeps same-language auto detection from requiring unrelated
    /// cloud credentials while preserving explicit provider choice.
    pub fn with_translation(
        mut self,
        translation: TranslationProvider,
        credentials: &Credentials,
    ) -> Result<Self, EditError> {
        let (translator, mode) = build_translator(&translation, credentials)?;
        self.translator = Some(translator);
        self.translation = Some(translation);
        self.translation_mode = Some(mode);
        Ok(self)
    }

    /// Test/application composition with already built adapters.
    pub fn from_parts(
        transcription: TranscriptionProvider,
        transcriber: Arc<dyn ai::Transcriber>,
        aligner: Option<Arc<dyn ai::Aligner>>,
        translation: Option<TranslationProvider>,
        translator: Option<Arc<dyn ai::Translator>>,
    ) -> Result<Self, EditError> {
        let translation_mode = translation.as_ref().map(|_| "context-window".into());
        let needs_alignment = aligner.is_some();
        Ok(Self {
            transcriber: TranscriberKind::Other(transcriber),
            aligner,
            translator: translator.map(TranslatorKind::Other),
            transcription,
            translation,
            transcription_mode: "transcribe".into(),
            translation_mode,
            needs_alignment,
            _leases: Vec::new(),
        })
    }

    pub fn transcription(&self) -> &TranscriptionProvider {
        &self.transcription
    }
    pub fn translation(&self) -> Option<&TranslationProvider> {
        self.translation.as_ref()
    }
    pub fn transcriber(&self) -> &dyn ai::Transcriber {
        self.transcriber.as_transcriber()
    }
    pub fn translator(&self) -> Option<&dyn ai::Translator> {
        self.translator.as_ref().map(TranslatorKind::as_translator)
    }
    fn translator_arc(&self) -> Option<Arc<dyn ai::Translator>> {
        self.translator
            .as_ref()
            .map(|value| -> Arc<dyn ai::Translator> {
                match value {
                    TranslatorKind::OpenAi(value) => value.clone(),
                    TranslatorKind::Anthropic(value) => value.clone(),
                    TranslatorKind::Other(value) => value.clone(),
                }
            })
    }
    fn transcriber_arc(&self) -> Arc<dyn ai::Transcriber> {
        match &self.transcriber {
            TranscriberKind::Whisper(value) => value.clone(),
            TranscriberKind::Gipformer(value) => value.clone(),
            TranscriberKind::OpenAi(value) => value.clone(),
            TranscriberKind::ElevenLabs(value) => value.clone(),
            TranscriberKind::Other(value) => value.clone(),
        }
    }
    fn aligner_arc(&self) -> Option<Arc<dyn ai::Aligner>> {
        self.aligner.clone()
    }
    pub(crate) fn language_profile(
        &self,
        language: &LanguageTag,
    ) -> Result<LanguageProfile, EditError> {
        Ok(LanguageRegistry::standard()?.profile(language).clone())
    }
    fn transcription_scope(
        &self,
        language: &LanguageTag,
        samples: SampleRange,
        sample_count: u64,
        payload: &NfcText,
        remote_bytes: u64,
    ) -> Result<Scope, EditError> {
        Ok(Scope {
            provider: self.transcriber().config().clone(),
            operation: ai::Operation::Transcribe,
            payload_fingerprint: payload.clone(),
            mode: self.transcription_mode.clone(),
            texts: Vec::new(),
            source_language: language.clone(),
            target_language: None,
            source_samples: samples,
            sample_rate: media::SPEECH_SAMPLE_RATE,
            max_bytes: remote_bytes.max(sample_count.saturating_mul(2).saturating_add(44)),
        })
    }
    fn alignment_scope(
        &self,
        language: &LanguageTag,
        samples: SampleRange,
        payload: &NfcText,
    ) -> Result<Scope, EditError> {
        let aligner = self
            .aligner
            .as_ref()
            .ok_or(EditError::Unsupported("missing aligner"))?;
        Ok(Scope {
            provider: aligner.config().clone(),
            operation: ai::Operation::Align,
            payload_fingerprint: payload.clone(),
            mode: ai::alignment::MODE.into(),
            texts: Vec::new(),
            source_language: language.clone(),
            target_language: None,
            source_samples: samples,
            sample_rate: media::SPEECH_SAMPLE_RATE,
            max_bytes: samples.len().saturating_mul(4),
        })
    }
    fn translation_scope(
        &self,
        source: &LanguageTag,
        target: &LanguageTag,
        payload: &NfcText,
        fingerprint: &NfcText,
    ) -> Result<Scope, EditError> {
        let translator = self
            .translator()
            .ok_or(EditError::Unsupported("missing translator"))?;
        Ok(Scope {
            provider: translator.config().clone(),
            operation: ai::Operation::Translate,
            payload_fingerprint: fingerprint.clone(),
            mode: self
                .translation_mode
                .clone()
                .ok_or(EditError::Unsupported("missing translation mode"))?,
            texts: vec![payload.clone()],
            source_language: source.clone(),
            target_language: Some(target.clone()),
            source_samples: SampleRange::new(0, 1)?,
            sample_rate: 1,
            max_bytes: u64::try_from(payload.as_str().len())
                .map_err(|_| EditError::Limit)?
                .max(MAX_JSON_BYTES as u64),
        })
    }
}

fn build_translator(
    translation: &TranslationProvider,
    credentials: &Credentials,
) -> Result<(TranslatorKind, NfcText), EditError> {
    match translation {
        TranslationProvider::OpenAi { model } => {
            let config = ai::ProviderConfig::new(
                "openai".into(),
                model.as_str().into(),
                ai::openai::TRANSLATION_SETTINGS.into(),
                Location::Remote {
                    backend: ai::openai::BACKEND.into(),
                    base_url: ai::openai::BASE_URL.into(),
                    auth_ref: "OPENAI_API_KEY".into(),
                },
            )?;
            let value = ai::openai::OpenAiTranslator::new(
                config,
                credentials.openai()?.clone(),
                Arc::new(UreqTransport::new()),
            )?;
            Ok((
                TranslatorKind::OpenAi(Arc::new(value)),
                ai::openai::TRANSLATION_MODE.into(),
            ))
        }
        TranslationProvider::Anthropic { model } => {
            let config = ai::ProviderConfig::new(
                "anthropic".into(),
                model.as_str().into(),
                ai::anthropic::TRANSLATION_SETTINGS.into(),
                Location::Remote {
                    backend: ai::anthropic::BACKEND.into(),
                    base_url: ai::anthropic::BASE_URL.into(),
                    auth_ref: "ANTHROPIC_API_KEY".into(),
                },
            )?;
            let value = ai::anthropic::AnthropicTranslator::new(
                config,
                credentials.anthropic()?.clone(),
                Arc::new(UreqTransport::new()),
            )?;
            Ok((
                TranslatorKind::Anthropic(Arc::new(value)),
                ai::anthropic::TRANSLATION_MODE.into(),
            ))
        }
    }
}

fn load_aligner(
    cache: &mut ArtifactCache,
    leases: &mut Vec<ModelLease>,
) -> Result<Arc<dyn ai::Aligner>, EditError> {
    #[cfg(feature = "candle-align")]
    {
        let metadata = ai::models::alignment_model()?;
        let lease = cache
            .model(&metadata, &IdleHooks)?
            .ok_or(EditError::ModelMissing("whisper-small-dtw"))?;
        let value = ai::alignment::CandleAligner::load(&metadata, lease.directory(), &IdleHooks)?;
        leases.push(lease);
        Ok(Arc::new(value))
    }
    #[cfg(not(feature = "candle-align"))]
    {
        let _ = (cache, leases);
        Err(EditError::Unsupported(
            "local alignment support is not enabled in this build",
        ))
    }
}

/// A provider disclosure shown before any remote byte leaves the machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cost {
    Free,
    UpperBound { currency: String, micros: u64 },
    Unknown,
}
impl Cost {
    pub(crate) fn add(&mut self, next: &CostEstimate) {
        match (&mut *self, next) {
            (Self::Unknown, _) => {}
            (_, CostEstimate::Unknown) => *self = Self::Unknown,
            (Self::Free, CostEstimate::Free) => {}
            (Self::Free, CostEstimate::UpperBound(money)) => {
                *self = Self::UpperBound {
                    currency: money.currency().into(),
                    micros: money.micros(),
                };
            }
            (Self::UpperBound { currency, micros }, CostEstimate::Free) => {
                let _ = (currency, micros);
            }
            (Self::UpperBound { currency, micros }, CostEstimate::UpperBound(money))
                if currency == money.currency() =>
            {
                *micros = micros.saturating_add(money.micros());
            }
            (Self::UpperBound { .. }, CostEstimate::UpperBound(_)) => *self = Self::Unknown,
        }
    }
}
impl From<&CostEstimate> for Cost {
    fn from(value: &CostEstimate) -> Self {
        match value {
            CostEstimate::Free => Self::Free,
            CostEstimate::UpperBound(money) => Self::UpperBound {
                currency: money.currency().into(),
                micros: money.micros(),
            },
            CostEstimate::Unknown => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug)]
pub struct UploadRequest {
    pub operation: CatalogOperation,
    pub provider: String,
    pub model: String,
    pub languages: String,
    pub audio_seconds: f64,
    pub upload_bytes: u64,
    pub cost: Cost,
    pub terms: Vec<(&'static str, String)>,
}
/// The adapter refuses the remote request when this returns `Err(ConsentRequired)`.
pub trait ConsentPrompt {
    fn approve(&self, request: &UploadRequest) -> Result<(), EditError>;
}
/// A prompt that refuses every remote request. Test fixtures and non-interactive
/// callers must opt in explicitly with [`AllowAll`].
pub struct DenyUploads;
impl ConsentPrompt for DenyUploads {
    fn approve(&self, _: &UploadRequest) -> Result<(), EditError> {
        Err(EditError::ConsentRequired)
    }
}
pub struct AllowAll;
impl ConsentPrompt for AllowAll {
    fn approve(&self, _: &UploadRequest) -> Result<(), EditError> {
        Ok(())
    }
}

/// Operation-wide cooperative cancellation shared by extraction, every submitted
/// job and the later assembly/translation/export stages.
#[derive(Clone, Default)]
pub struct Cancellation {
    flag: Arc<AtomicBool>,
}
impl Cancellation {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.flag.store(true, std::sync::atomic::Ordering::Release);
    }
    pub fn cancelled(&self) -> bool {
        self.flag.load(std::sync::atomic::Ordering::Acquire)
    }
    pub(crate) fn media_flag(&self) -> Arc<AtomicBool> {
        self.flag.clone()
    }
    pub(crate) fn checkpoint(&self) -> Result<(), EditError> {
        if self.cancelled() {
            Err(EditError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Extract,
    Transcribe,
    Align,
    Assemble,
    Segment,
    SuggestClips,
    Snap,
    Translate,
    ExportSrt,
    ExportBurn,
}
/// Progress is presentation only; the engine owns job state and checkpoint truth.
pub trait Progress {
    fn stage(&self, stage: Stage);
    fn job(&self, snapshot: &JobSnapshot) {
        let _ = snapshot;
    }
}
pub struct SilentProgress;
impl Progress for SilentProgress {
    fn stage(&self, _: Stage) {}
}

#[derive(Clone, Debug)]
pub struct SubtitleRequest {
    pub input: PathBuf,
    /// `None` requests local whisper.cpp language detection.
    pub source_language: Option<LanguageTag>,
    pub target_language: LanguageTag,
    pub bilingual: bool,
    pub srt: Option<PathBuf>,
    pub burn: Option<PathBuf>,
    /// `Some(true)` selects 1080x1920; `Some(false)` selects 1920x1080; `None` keeps
    /// the project canvas.
    pub portrait: Option<bool>,
    pub overwrite: bool,
}

#[derive(Clone, Debug)]
pub struct SubtitleOutcome {
    pub source_language: LanguageTag,
    pub target_language: LanguageTag,
    pub transcription: TranscriptionProvider,
    pub translation: Option<TranslationProvider>,
    pub segments: usize,
    pub cues: usize,
    pub checkpoints: usize,
    pub srt: Option<PathBuf>,
    pub burn: Option<PathBuf>,
    pub transcription_cost: Cost,
    pub translation_cost: Cost,
    pub elapsed: Duration,
}

/// Resolved speech recognition plus the source-timed cues shared by the subtitle
/// and clip workflows. No translation or file publication happens here.
pub struct Transcription {
    pub source_language: LanguageTag,
    pub transcript: Transcript,
    pub cues: Vec<Cue>,
    pub cost: Cost,
    pub checkpoints: usize,
    pub elapsed: Duration,
}

/// Run the shared ASR-to-cue stage. The caller has already imported and validated
/// media and resolved the source language; jobs, cache and consent stay caller-owned.
#[allow(clippy::too_many_arguments)]
pub fn transcribe(
    asset: &SourceAsset,
    duration: Tick,
    source_language: &LanguageTag,
    providers: &Providers,
    runner: &mut JobRunner,
    cache: &mut ArtifactCache,
    consent: &dyn ConsentPrompt,
    progress: &dyn Progress,
    cancel: &Cancellation,
) -> Result<Transcription, EditError> {
    if !["vi", "en", "zh"]
        .iter()
        .any(|tag| *tag == source_language.as_str())
    {
        return Err(EditError::Unsupported(
            "source language must be vi, en or zh",
        ));
    }
    cancel.checkpoint()?;
    let started = Instant::now();
    let profile = providers.language_profile(source_language)?;
    let speech_settings = SpeechExtractSettings {
        // Text-only providers are timed by the local 30 s DTW aligner.
        max_duration: Tick::from_seconds_f64(
            if providers.needs_alignment {
                MIN_CAPTION_SECONDS as f64
            } else {
                MAX_CAPTION_SECONDS as f64
            },
            Rounding::Floor,
        )?,
        overlap: Tick::ZERO,
        max_bytes: WORK_BYTES,
        bit_rate: 48_000,
    };
    let full_range = SourceRange::new(Tick::ZERO, duration)?;
    let fingerprint = NfcText::from(hex(&asset.content_hash().0));
    let mut recognitions = Vec::new();
    let mut checkpoints = 0usize;
    let mut cost = Cost::Free;
    let mut failure = None;
    cancel.checkpoint()?;
    progress.stage(Stage::Extract);
    let extraction = media::extract_speech_windows_with_cancel(
        asset,
        full_range,
        &speech_settings,
        cancel.media_flag(),
        |window| {
            if cancel.cancelled() {
                return Err(media::MediaError::Cancelled);
            }
            if !window.speech_detected {
                return Ok(());
            }
            progress.stage(Stage::Transcribe);
            match transcribe_window(
                providers,
                runner,
                cache,
                consent,
                progress,
                cancel,
                source_language,
                &fingerprint,
                window,
                &mut cost,
                &mut checkpoints,
            ) {
                Ok(recognition) => {
                    recognitions.push(recognition);
                    Ok(())
                }
                Err(error) => {
                    failure = Some(error);
                    Err(media::MediaError::Cancelled)
                }
            }
        },
    );
    if let Some(error) = failure {
        return Err(error);
    }
    match extraction {
        Ok(()) => {}
        Err(media::MediaError::Cancelled) => return Err(EditError::Cancelled),
        Err(error) => return Err(error.into()),
    }
    cancel.checkpoint()?;
    if recognitions.is_empty() {
        return Err(EditError::Unsupported("no speech detected"));
    }

    cancel.checkpoint()?;
    progress.stage(Stage::Assemble);
    let recognition = ai::assemble(recognitions, &IdleHooks)?;
    let transcript = recognition.to_transcript()?;
    cancel.checkpoint()?;
    progress.stage(Stage::Segment);
    let rules = SegmentRules::from_profile(&profile, FrameRate::FPS_30)?;
    let segmentation = captions::segment_transcript(
        &transcript,
        &profile,
        &rules,
        &captions::LayoutHooks::default(),
    )?;
    let cues: Vec<Cue> = segmentation
        .cues
        .into_iter()
        .map(|SegmentedCue { cue, .. }| cue)
        .collect();
    if cues.is_empty() {
        return Err(EditError::Unsupported("no subtitle cues generated"));
    }
    Ok(Transcription {
        source_language: source_language.clone(),
        transcript,
        cues,
        cost,
        checkpoints,
        elapsed: started.elapsed(),
    })
}

/// Run the complete subtitle workflow against one input media file. The caller owns
/// the runner, cache and consent prompt; no project file is written.
pub fn run(
    request: &SubtitleRequest,
    providers: &Providers,
    runner: &mut JobRunner,
    cache: &mut ArtifactCache,
    consent: &dyn ConsentPrompt,
    progress: &dyn Progress,
    cancel: &Cancellation,
) -> Result<SubtitleOutcome, EditError> {
    let started = Instant::now();
    cancel.checkpoint()?;
    if request.srt.is_none() && request.burn.is_none() {
        return Err(EditError::Invalid("no subtitle output requested"));
    }
    let asset = SourceAsset::import(media::AssetId(1), request.input.clone())?;
    if asset
        .probe()
        .duration
        .is_none_or(|value| value <= Tick::ZERO)
    {
        return Err(EditError::Unsupported("media duration is unknown"));
    }
    if request.burn.is_some()
        && !asset
            .probe()
            .streams
            .iter()
            .any(|stream| stream.kind == media::StreamKind::Video)
    {
        return Err(EditError::Unsupported("burn-in requires a video stream"));
    }
    let duration = media_duration(&asset)?;
    let source_language = match &request.source_language {
        Some(language) => language.clone(),
        None => providers.detect_language(&asset, duration, cancel)?,
    };
    let target_language = normalize_target(&request.target_language)?;
    if request.bilingual && source_language == target_language {
        return Err(EditError::Unsupported(
            "bilingual output requires two distinct languages",
        ));
    }
    let need_translation = source_language != target_language;
    if need_translation && providers.translator().is_none() {
        return Err(EditError::Unsupported("translation provider is required"));
    }
    if !need_translation && request.bilingual {
        return Err(EditError::Unsupported(
            "bilingual output requires a translation target",
        ));
    }
    let transcription = transcribe(
        &asset,
        duration,
        &source_language,
        providers,
        runner,
        cache,
        consent,
        progress,
        cancel,
    )?;
    let cues = transcription.cues;
    let transcription_cost = transcription.cost;
    let checkpoints = transcription.checkpoints;
    let segments = transcription.transcript.segments.len();
    let fingerprint = NfcText::from(hex(&asset.content_hash().0));
    cancel.checkpoint()?;
    let mut session = build_session(
        &asset,
        &cues,
        duration,
        request.burn.is_some(),
        request.portrait,
    )?;
    let caption_item = caption_item_id(&session)?;
    let caption_track = caption_track_id(&session)?;
    let mut translation_cost = Cost::Free;
    if need_translation {
        progress.stage(Stage::Translate);
        translate_cues(
            &mut session,
            providers,
            runner,
            consent,
            progress,
            cancel,
            &source_language,
            &target_language,
            &fingerprint,
            caption_item,
            &mut translation_cost,
        )?;
    }
    if let (Some(srt), Some(burn)) = (&request.srt, &request.burn)
        && resolve_path(srt)? == resolve_path(burn)?
    {
        return Err(EditError::Invalid("subtitle outputs must differ"));
    }
    let output_languages = if need_translation && request.bilingual {
        vec![source_language.clone(), target_language.clone()]
    } else {
        vec![target_language.clone()]
    };
    let srt = match &request.srt {
        Some(path) => {
            ensure_distinct(&request.input, path)?;
            cancel.checkpoint()?;
            progress.stage(Stage::ExportSrt);
            export_subtitles(
                &session,
                runner,
                progress,
                &asset,
                caption_track,
                &output_languages,
                path,
                request.overwrite,
                cancel,
            )?;
            Some(path.clone())
        }
        None => None,
    };
    let burn = match &request.burn {
        Some(path) => {
            ensure_distinct(&request.input, path)?;
            cancel.checkpoint()?;
            progress.stage(Stage::ExportBurn);
            burn_subtitles(
                &session,
                runner,
                progress,
                &asset,
                duration,
                &output_languages,
                path,
                request.portrait,
                request.overwrite,
                cancel,
            )?;
            Some(path.clone())
        }
        None => None,
    };
    Ok(SubtitleOutcome {
        source_language,
        target_language,
        transcription: providers.transcription().clone(),
        translation: providers.translation().cloned(),
        segments,
        cues: cues.len(),
        checkpoints,
        srt,
        burn,
        transcription_cost,
        translation_cost,
        elapsed: started.elapsed(),
    })
}

impl Providers {
    /// Import a media path and detect the spoken language without opening a
    /// translator. Composition roots use this to decide whether translation (and
    /// its credentials) is required at all.
    pub fn detect_language_for(
        &self,
        input: &Path,
        cancel: &Cancellation,
    ) -> Result<LanguageTag, EditError> {
        cancel.checkpoint()?;
        let asset = SourceAsset::import(media::AssetId(1), input.to_path_buf())?;
        let duration = media_duration(&asset)?;
        self.detect_language(&asset, duration, cancel)
    }

    fn detect_language(
        &self,
        asset: &SourceAsset,
        duration: Tick,
        cancel: &Cancellation,
    ) -> Result<LanguageTag, EditError> {
        cancel.checkpoint()?;
        let whisper = self
            .transcriber
            .whisper()
            .ok_or(EditError::AutoLanguageUnsupported)?;
        let end = duration.min(Tick::from_seconds_f64(
            MIN_CAPTION_SECONDS as f64,
            Rounding::Floor,
        )?);
        let range = SourceRange::new(Tick::ZERO, end)?;
        let settings = SpeechExtractSettings {
            max_duration: end,
            overlap: Tick::ZERO,
            max_bytes: WORK_BYTES,
            bit_rate: 48_000,
        };
        let mut detected = None;
        let mut failure = None;
        let result = media::extract_speech_windows_with_cancel(
            asset,
            range,
            &settings,
            cancel.media_flag(),
            |window| {
                if cancel.cancelled() {
                    return Err(media::MediaError::Cancelled);
                }
                match detection_window(&window).and_then(|(samples, range)| {
                    whisper
                        .detect_language(
                            ai::AudioWindow {
                                samples,
                                range,
                                sample_rate: media::SPEECH_SAMPLE_RATE,
                            },
                            &IdleHooks,
                        )
                        .map_err(EditError::Ai)
                }) {
                    Ok(language) => {
                        detected = Some(language);
                        Ok(())
                    }
                    Err(error) => {
                        failure = Some(error);
                        Err(media::MediaError::Cancelled)
                    }
                }
            },
        );
        if let Some(error) = failure {
            return Err(error);
        }
        match result {
            Ok(()) => {}
            Err(media::MediaError::Cancelled) => return Err(EditError::Cancelled),
            Err(error) => return Err(error.into()),
        }
        cancel.checkpoint()?;
        detected.ok_or(EditError::Unsupported(
            "no speech detected for language detection",
        ))
    }
}

fn detection_window(window: &SpeechWindow) -> Result<(&[f32], SampleRange), EditError> {
    let (start, end) = sample_range(window)?;
    Ok((window.samples.as_slice(), SampleRange::new(start, end)?))
}

fn normalize_target(target: &LanguageTag) -> Result<LanguageTag, EditError> {
    if target.as_str() == "zh" {
        Ok(LanguageTag::parse("zh-Hans")?)
    } else {
        Ok(target.clone())
    }
}

fn media_duration(asset: &SourceAsset) -> Result<Tick, EditError> {
    asset
        .probe()
        .duration
        .filter(|value| *value > Tick::ZERO)
        .ok_or(EditError::Unsupported("media duration is unknown"))
}

fn sample_range(window: &SpeechWindow) -> Result<(u64, u64), EditError> {
    let start = window
        .source_range
        .start()
        .to_units(i64::from(media::SPEECH_SAMPLE_RATE), Rounding::Nearest)?;
    let start = u64::try_from(start).map_err(|_| EditError::Invalid("sample origin"))?;
    let size = u64::try_from(window.samples.len()).map_err(|_| EditError::Limit)?;
    let end = start.checked_add(size).ok_or(EditError::Limit)?;
    Ok((start, end))
}

#[allow(clippy::too_many_arguments)]
fn transcribe_window(
    providers: &Providers,
    runner: &mut JobRunner,
    cache: &mut ArtifactCache,
    consent: &dyn ConsentPrompt,
    progress: &dyn Progress,
    cancel: &Cancellation,
    language: &LanguageTag,
    fingerprint: &NfcText,
    window: SpeechWindow,
    cost: &mut Cost,
    checkpoints: &mut usize,
) -> Result<ai::Recognition, EditError> {
    cancel.checkpoint()?;
    let (start, end) = sample_range(&window)?;
    let samples = SampleRange::new(start, end)?;
    let sample_count = u64::try_from(window.samples.len()).map_err(|_| EditError::Limit)?;
    let samples_pcm = Arc::new(window.samples);
    let scope = providers.transcription_scope(language, samples, sample_count, fingerprint, 0)?;
    let config = providers.transcriber().config();
    let key = CacheKey::new(KeyInputs {
        kind: ArtifactKind::AsrChunk,
        source: *blake3::hash(
            format!("{}:{}:{}", hex(fingerprint.as_str().as_bytes()), start, end).as_bytes(),
        )
        .as_bytes(),
        provider: config.id().as_str(),
        model: config.model().as_str(),
        version: config.settings_fingerprint().as_str(),
        settings: format!("language={};samples={start}..{end}", language.as_str()).as_bytes(),
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })?;
    let cached = match cache.get(key) {
        Ok(value) => value,
        Err(EditError::CacheCorrupt) => None,
        Err(error) => return Err(error),
    };
    let raw = match cached {
        Some(lease) => {
            *checkpoints = checkpoints.saturating_add(1);
            serde_json::from_slice(lease.bytes())
                .map_err(|_| EditError::Invalid("cached recognition is malformed"))?
        }
        None => {
            let estimate = providers.transcriber().estimate(&scope);
            cost.add(&estimate);
            let consent_token = if providers.transcription().is_remote() {
                let request = providers.upload_request(scope.clone())?;
                consent.approve(&request)?;
                Some(ai::Consent::grant(
                    scope.clone(),
                    estimate_money(&estimate)?,
                )?)
            } else {
                None
            };
            let provider = providers.transcriber_arc();
            let job_scope = scope.clone();
            let job_samples = samples_pcm.clone();
            let (id, token) = runner.submit(
                JobSpec {
                    kind: JobKind::Transcribe,
                    class: ResourceClass::Network,
                    input: key,
                    bytes: JobBytes {
                        compositor: WORK_BYTES,
                        result: 16 * 1024 * 1024,
                        ..Default::default()
                    },
                },
                move |hooks| {
                    let audio = ai::AudioWindow {
                        samples: job_samples.as_slice(),
                        range: job_scope.source_samples,
                        sample_rate: job_scope.sample_rate,
                    };
                    let request = Request {
                        scope: &job_scope,
                        consent: consent_token.as_ref(),
                    };
                    let recognition =
                        ai::transcribe(provider.as_ref(), audio, &request, None, hooks)?;
                    serde_json::to_vec(&recognition)
                        .map_err(|_| EditError::Invalid("recognition serialization"))
                },
            )?;
            let output = wait_for_job(runner, id, &token, progress, cancel)?;
            let bytes = output.bytes().to_vec();
            let recognition: ai::Recognition = serde_json::from_slice(&bytes)
                .map_err(|_| EditError::Invalid("recognition response is malformed"))?;
            match cache.put(key, &bytes) {
                Ok(()) => {}
                // A checkpoint that cannot be stored must not discard a valid recognition.
                Err(EditError::BudgetFull | EditError::ArtifactTooLarge) => {}
                Err(error) => return Err(error),
            }
            if cancel.cancelled() {
                return Err(EditError::Cancelled);
            }
            return align_recognition(
                providers,
                runner,
                progress,
                cancel,
                language,
                fingerprint,
                &samples_pcm,
                samples,
                recognition,
            );
        }
    };
    align_recognition(
        providers,
        runner,
        progress,
        cancel,
        language,
        fingerprint,
        &samples_pcm,
        samples,
        raw,
    )
}

#[allow(clippy::too_many_arguments)]
fn align_recognition(
    providers: &Providers,
    runner: &mut JobRunner,
    progress: &dyn Progress,
    cancel: &Cancellation,
    language: &LanguageTag,
    fingerprint: &NfcText,
    samples_pcm: &Arc<Vec<f32>>,
    samples: SampleRange,
    recognition: ai::Recognition,
) -> Result<ai::Recognition, EditError> {
    let Some(aligner) = providers.aligner_arc() else {
        return Ok(recognition);
    };
    let unit = providers.language_profile(language)?.timing_unit;
    if !recognition.needs_timing(unit) {
        return Ok(recognition);
    }
    cancel.checkpoint()?;
    progress.stage(Stage::Align);
    let scope = providers.alignment_scope(language, samples, fingerprint)?;
    let samples_pcm = samples_pcm.clone();
    let aligner_job = aligner.clone();
    let scope_job = scope.clone();
    let (id, token) = runner.submit(
        JobSpec {
            kind: JobKind::Transcribe,
            class: ResourceClass::Cpu,
            input: CacheKey::new(KeyInputs {
                kind: ArtifactKind::AsrChunk,
                source: *blake3::hash(
                    format!(
                        "align:{}:{}:{}",
                        fingerprint.as_str(),
                        samples.start(),
                        samples.end()
                    )
                    .as_bytes(),
                )
                .as_bytes(),
                provider: aligner.config().id().as_str(),
                model: aligner.config().model().as_str(),
                version: aligner.config().settings_fingerprint().as_str(),
                settings: format!("language={}", language.as_str()).as_bytes(),
                transcript: [0; 32],
                cues: [0; 32],
                clip_range: [0; 32],
                translation: [0; 32],
            })?,
            bytes: JobBytes {
                compositor: WORK_BYTES,
                result: 16 * 1024 * 1024,
                ..Default::default()
            },
        },
        move |hooks| {
            let audio = ai::AudioWindow {
                samples: samples_pcm.as_slice(),
                range: scope_job.source_samples,
                sample_rate: scope_job.sample_rate,
            };
            let aligned = ai::align(
                recognition.clone(),
                aligner_job.as_ref(),
                audio,
                &scope_job,
                None,
                unit,
                hooks,
            )?;
            serde_json::to_vec(&aligned)
                .map_err(|_| EditError::Invalid("aligned recognition serialization"))
        },
    )?;
    let output = wait_for_job(runner, id, &token, progress, cancel)?;
    if cancel.cancelled() {
        return Err(EditError::Cancelled);
    }
    serde_json::from_slice(output.bytes())
        .map_err(|_| EditError::Invalid("aligned recognition is malformed"))
}

impl Providers {
    fn upload_request(&self, scope: Scope) -> Result<UploadRequest, EditError> {
        match &self.transcriber {
            TranscriberKind::OpenAi(value) => {
                let disclosure = value.disclosure(&scope)?;
                Ok(UploadRequest {
                    operation: CatalogOperation::Transcribe,
                    provider: disclosure.provider.as_str().into(),
                    model: disclosure.model.as_str().into(),
                    languages: disclosure.language.as_str().into(),
                    audio_seconds: disclosure.audio_micros as f64 / 1_000_000.0,
                    upload_bytes: disclosure.upload_bytes,
                    cost: Cost::from(&disclosure.estimate),
                    terms: vec![
                        ("policy", disclosure.policy_url.as_str().into()),
                        ("docs", disclosure.docs_url.as_str().into()),
                    ],
                })
            }
            TranscriberKind::ElevenLabs(value) => {
                let disclosure = value.disclosure(&scope)?;
                Ok(UploadRequest {
                    operation: CatalogOperation::Transcribe,
                    provider: disclosure.provider.as_str().into(),
                    model: disclosure.model.as_str().into(),
                    languages: disclosure.language.as_str().into(),
                    audio_seconds: disclosure.audio_micros as f64 / 1_000_000.0,
                    upload_bytes: disclosure.upload_bytes,
                    cost: Cost::from(&disclosure.estimate),
                    terms: vec![
                        ("policy", disclosure.policy_url.as_str().into()),
                        ("pricing", disclosure.pricing_url.as_str().into()),
                        ("docs", disclosure.docs_url.as_str().into()),
                    ],
                })
            }
            TranscriberKind::Other(value) => {
                let estimate = value.estimate(&scope);
                Ok(UploadRequest {
                    operation: CatalogOperation::Transcribe,
                    provider: value.config().id().as_str().into(),
                    model: value.config().model().as_str().into(),
                    languages: scope.source_language.as_str().into(),
                    audio_seconds: scope.source_samples.len() as f64 / f64::from(scope.sample_rate),
                    upload_bytes: scope.max_bytes,
                    cost: Cost::from(&estimate),
                    terms: vec![("policy", "test provider policy".into())],
                })
            }
            _ => Err(EditError::Unsupported("local provider has no upload")),
        }
    }
    fn translation_upload_request(&self, scope: Scope) -> Result<UploadRequest, EditError> {
        match &self.translator {
            Some(TranslatorKind::OpenAi(value)) => {
                let disclosure = value.disclosure(&scope)?;
                Ok(UploadRequest {
                    operation: CatalogOperation::Translate,
                    provider: "openai".into(),
                    model: disclosure.model.into(),
                    languages: format!(
                        "{} -> {}",
                        disclosure.source_language.as_str(),
                        disclosure.target_language.as_str()
                    ),
                    audio_seconds: 0.0,
                    upload_bytes: u64::try_from(
                        scope.texts.first().map_or(0, |t| t.as_str().len()),
                    )
                    .map_err(|_| EditError::Limit)?,
                    cost: Cost::from(&disclosure.estimate),
                    terms: vec![("policy", disclosure.policy_url.into())],
                })
            }
            Some(TranslatorKind::Anthropic(value)) => {
                let disclosure = value.disclosure(&scope)?;
                Ok(UploadRequest {
                    operation: CatalogOperation::Translate,
                    provider: disclosure.provider.into(),
                    model: disclosure.model.into(),
                    languages: format!(
                        "{} -> {}",
                        disclosure.source_language.as_str(),
                        disclosure.target_language.as_str()
                    ),
                    audio_seconds: 0.0,
                    upload_bytes: u64::try_from(
                        scope.texts.first().map_or(0, |t| t.as_str().len()),
                    )
                    .map_err(|_| EditError::Limit)?,
                    cost: Cost::from(&disclosure.estimate),
                    terms: vec![
                        ("policy", disclosure.policy_url.into()),
                        ("pricing", disclosure.pricing_url.into()),
                        ("structured_output", disclosure.structured_output_url.into()),
                    ],
                })
            }
            Some(TranslatorKind::Other(value)) => {
                let estimate = value.estimate(&scope);
                Ok(UploadRequest {
                    operation: CatalogOperation::Translate,
                    provider: value.config().id().as_str().into(),
                    model: value.config().model().as_str().into(),
                    languages: format!(
                        "{} -> {}",
                        scope.source_language.as_str(),
                        scope
                            .target_language
                            .as_ref()
                            .map_or("", LanguageTag::as_str)
                    ),
                    audio_seconds: 0.0,
                    upload_bytes: scope.max_bytes,
                    cost: Cost::from(&estimate),
                    terms: vec![("policy", "test provider policy".into())],
                })
            }
            None => Err(EditError::Unsupported("missing translator")),
        }
    }
}

pub(crate) fn wait_for_job(
    runner: &mut JobRunner,
    id: JobId,
    token: &CancelToken,
    progress: &dyn Progress,
    cancel: &Cancellation,
) -> Result<JobOutput, EditError> {
    loop {
        if cancel.cancelled() {
            token.cancel();
        }
        runner.tick()?;
        for event in runner.drain_events() {
            if event.id == id {
                progress.job(&event);
            }
        }
        let snapshot = runner.snapshot(id).ok_or(EditError::StaleResult)?;
        match snapshot.state {
            // A completed job is returned even if cancellation arrived during its
            // publication fence; callers decide whether later stages may proceed.
            JobState::Completed => return runner.take_output(id).ok_or(EditError::StaleResult),
            JobState::Failed => {
                return Err(match snapshot.error {
                    Some(error) => EditError::JobFailure(error),
                    None => EditError::Invalid("job failed without diagnostics"),
                });
            }
            JobState::Cancelled => return Err(EditError::Cancelled),
            _ => {
                if cancel.cancelled() {
                    token.cancel();
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

pub(crate) fn build_session(
    asset: &SourceAsset,
    cues: &[Cue],
    duration: Tick,
    with_video: bool,
    portrait: Option<bool>,
) -> Result<Session, EditError> {
    let mut data = Project::empty(ProjectId(1), "Subtitles".into())?.into_data();
    if with_video
        && let Some(video) = asset
            .probe()
            .streams
            .iter()
            .find(|stream| stream.kind == media::StreamKind::Video)
    {
        if let Some(info) = &video.video {
            data.canvas.width = info.width;
            data.canvas.height = info.height;
        }
        if let Some(rate) = video.frame_rate {
            data.canvas.fps = rate;
        }
    }
    if with_video && portrait == Some(false) {
        data.canvas.width = 1920;
        data.canvas.height = 1080;
    }
    let mut session = Session::new(Project::new(data)?, 16)?;
    session.apply(Command::AddAsset {
        asset: asset.clone(),
    })?;
    if with_video {
        let video = session.project().data().scene().timeline.main.id;
        let range = SourceRange::new(Tick::ZERO, duration)?;
        session.apply(Command::InsertItem {
            placement: crate::Placement::Track { track: video },
            item: NewItem {
                animations: Animations::default(),
                source_asset: Some(asset.id()),
                source_range: range,
                start: Tick::ZERO,
                content: ItemParams::Video(project::VideoParams {
                    source_audio_enabled: true,
                }),
            },
        })?;
    }
    let captions = track_id(session.apply(Command::AddTrack {
        placement: crate::TrackPlacement::Auto,
        name: "Captions".into(),
        kind: TrackKind::Text,
    })?)?;
    let range = SourceRange::new(Tick::ZERO, duration)?;
    session.apply(Command::InsertItem {
        placement: crate::Placement::Track { track: captions },
        item: NewItem {
            animations: Animations::default(),
            source_asset: Some(asset.id()),
            source_range: range,
            start: Tick::ZERO,
            content: ItemParams::Caption(CaptionParams {
                cues: CueTrack {
                    cues: cues.to_vec(),
                },
            }),
        },
    })?;
    Ok(session)
}

fn track_id(outcome: Outcome) -> Result<TrackId, EditError> {
    outcome
        .created
        .into_iter()
        .find_map(|created| match created {
            crate::CreatedId::Track(id) => Some(id),
            _ => None,
        })
        .ok_or(EditError::Invalid("track was not created"))
}
fn caption_track_id(session: &Session) -> Result<TrackId, EditError> {
    session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .find(|track| {
            track
                .items
                .iter()
                .any(|item| matches!(item.content, ItemParams::Caption(_)))
        })
        .map(|track| track.id)
        .ok_or(EditError::Invalid("missing caption track"))
}
pub(crate) fn caption_item_id(session: &Session) -> Result<project::ItemId, EditError> {
    session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .filter(|track| track.kind == TrackKind::Text)
        .flat_map(|track| &track.items)
        .find(|item| matches!(item.content, ItemParams::Caption(_)))
        .map(|item| item.id)
        .ok_or(EditError::Invalid("missing caption item"))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn translate_cues(
    session: &mut Session,
    providers: &Providers,
    runner: &mut JobRunner,
    consent: &dyn ConsentPrompt,
    progress: &dyn Progress,
    cancel: &Cancellation,
    source: &LanguageTag,
    target: &LanguageTag,
    fingerprint: &NfcText,
    item: project::ItemId,
    cost: &mut Cost,
) -> Result<(), EditError> {
    let ids: Vec<NfcText> = {
        let project = session.project();
        let item = project
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|value| value.id == item)
            .ok_or(EditError::NoItem(item))?;
        let ItemParams::Caption(params) = &item.content else {
            return Err(EditError::Invalid("translation requires caption item"));
        };
        params.cues.cues.iter().map(|cue| cue.id.clone()).collect()
    };
    let translator = providers
        .translator()
        .ok_or(EditError::Unsupported("missing translator"))?;
    for chunk in ids.chunks(ai::MAX_SEGMENTS.min(64)) {
        cancel.checkpoint()?;
        let window = crate::translation::window(
            session,
            item,
            chunk,
            target.clone(),
            Vec::new(),
            "subtitle".into(),
            false,
        )?;
        let payload = window.consent_text()?;
        let scope = providers.translation_scope(source, target, &payload, fingerprint)?;
        let estimate = translator.estimate(&scope);
        cost.add(&estimate);
        let consent_token = Some(ai::Consent::grant(
            scope.clone(),
            estimate_money(&estimate)?,
        )?);
        let request = providers.translation_upload_request(scope.clone())?;
        consent.approve(&request)?;
        let run = crate::translation::TranslationRun::new(
            crate::translation::TranslationTarget { session, item },
            providers
                .translator_arc()
                .ok_or(EditError::Unsupported("missing translator"))?,
            window,
            scope,
            consent_token,
            TRANSLATE_ATTEMPTS,
            {
                let each = estimate_money(&estimate)?;
                Money::new(
                    each.currency(),
                    each.micros().saturating_mul(u64::from(TRANSLATE_ATTEMPTS)),
                )?
            },
        )?;
        let (id, token) = run.submit(runner, session)?;
        let output = wait_for_job(runner, id, &token, progress, cancel)?;
        cancel.checkpoint()?;
        let translation: ai::Translation = serde_json::from_slice(output.bytes())
            .map_err(|_| EditError::Invalid("translation response is malformed"))?;
        output.apply(
            session,
            Command::Retranslate {
                item,
                target_language: target.clone(),
                texts: translation.texts,
                translation_ref: format!(
                    "{}/{}",
                    providers.translation().map_or("", |p| p.id()),
                    providers.translation().map_or("", |p| p.model())
                )
                .into(),
                force: false,
            },
        )?;
    }
    Ok(())
}

/// Translate one source-timed cue set into a target language without writing a
/// project. Reuses the subtitle translation window/consent/job path.
#[allow(clippy::too_many_arguments)]
pub(crate) fn translate_track(
    asset: &SourceAsset,
    duration: Tick,
    cues: &[Cue],
    providers: &Providers,
    runner: &mut JobRunner,
    consent: &dyn ConsentPrompt,
    progress: &dyn Progress,
    cancel: &Cancellation,
    source: &LanguageTag,
    target: &LanguageTag,
    fingerprint: &NfcText,
    cost: &mut Cost,
) -> Result<CueTrack, EditError> {
    if providers.translator().is_none() {
        return Err(EditError::Unsupported("translation provider is required"));
    }
    let mut session = build_session(asset, cues, duration, false, None)?;
    let item = caption_item_id(&session)?;
    progress.stage(Stage::Translate);
    translate_cues(
        &mut session,
        providers,
        runner,
        consent,
        progress,
        cancel,
        source,
        target,
        fingerprint,
        item,
        cost,
    )?;
    session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find_map(|item| match &item.content {
            ItemParams::Caption(params) => Some(params.cues.clone()),
            _ => None,
        })
        .ok_or(EditError::Invalid("missing translated cues"))
}

pub(crate) fn estimate_money(estimate: &CostEstimate) -> Result<Money, EditError> {
    match estimate {
        CostEstimate::Free => Ok(Money::new("USD", 0)?),
        CostEstimate::UpperBound(money) => Ok(money.clone()),
        CostEstimate::Unknown => Err(EditError::Ai(ai::Error::Cost)),
    }
}

#[allow(clippy::too_many_arguments)]
fn export_subtitles(
    session: &Session,
    runner: &mut JobRunner,
    progress: &dyn Progress,
    asset: &SourceAsset,
    track: TrackId,
    languages: &[LanguageTag],
    path: &Path,
    overwrite: bool,
    cancel: &Cancellation,
) -> Result<(), EditError> {
    cancel.checkpoint()?;
    let key = export_key(asset, "srt")?;
    let (id, token) = session.submit_export_command(
        runner,
        key,
        Command::ExportSubtitles {
            track,
            format: captions::Format::Srt,
            languages: languages.to_vec(),
            path: path.to_path_buf(),
            overwrite,
        },
    )?;
    let _ = wait_for_job(runner, id, &token, progress, cancel)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn burn_subtitles(
    session: &Session,
    runner: &mut JobRunner,
    progress: &dyn Progress,
    asset: &SourceAsset,
    duration: Tick,
    languages: &[LanguageTag],
    path: &Path,
    portrait: Option<bool>,
    overwrite: bool,
    cancel: &Cancellation,
) -> Result<(), EditError> {
    cancel.checkpoint()?;
    let key = export_key(asset, "burn")?;
    let range = TimeRange::new(Tick::ZERO, duration)?;
    let (id, token) = session.submit_export_command(
        runner,
        key,
        Command::BurnSubtitles {
            range,
            languages: languages.to_vec(),
            style: render::CaptionStyle::default(),
            portrait: portrait.unwrap_or(false),
            path: path.to_path_buf(),
            overwrite,
        },
    )?;
    let _ = wait_for_job(runner, id, &token, progress, cancel)?;
    Ok(())
}

fn export_key(asset: &SourceAsset, label: &str) -> Result<CacheKey, EditError> {
    let settings = format!("subtitle-cli-v1:{label}");
    CacheKey::new(KeyInputs {
        kind: ArtifactKind::Preview,
        source: asset.content_hash().0,
        provider: "vibecut-cli",
        model: "subtitle",
        version: "1",
        settings: settings.as_bytes(),
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
}

fn ensure_distinct(input: &Path, output: &Path) -> Result<(), EditError> {
    if resolve_path(input)? == resolve_path(output)? {
        return Err(EditError::Invalid(
            "output must not replace the input media",
        ));
    }
    Ok(())
}
fn resolve_path(path: &Path) -> Result<PathBuf, EditError> {
    match std::fs::canonicalize(path) {
        Ok(value) => Ok(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let name = path
                .file_name()
                .ok_or(EditError::Invalid("output path has no file name"))?;
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            Ok(std::fs::canonicalize(parent)?.join(name))
        }
        Err(error) => Err(error.into()),
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(
            DIGITS.get(usize::from(byte >> 4)).copied().unwrap_or(b'0'),
        ));
        output.push(char::from(
            DIGITS
                .get(usize::from(byte & 0x0f))
                .copied()
                .unwrap_or(b'0'),
        ));
    }
    output
}
