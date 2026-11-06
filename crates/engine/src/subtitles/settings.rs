//! Resolves subtitle provider settings.
use super::{
    AllowAll, ApiKey, Arc, ArtifactCache, ByteBudget, CatalogOperation, ConsentPrompt, Cost,
    CostEstimate, Credentials, DenyUploads, EditError, ExecutionHooks, IdleHooks, JobId, JobRunner,
    LanguageProfile, LanguageRegistry, LanguageTag, Location, MAX_JSON_BYTES, ModelDownloader,
    ModelFileInfo, ModelInfo, ModelLease, ModelTransfers, Money, NfcText, Path, PathBuf,
    ProviderOption, Providers, Runtime, SampleRange, Scope, TranscriberKind, TranscriptionProvider,
    TranslationProvider, TranslatorKind, UploadRequest, UreqTransport,
};
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
    pub(super) fn from_model(model: &ai::models::Model) -> Result<Self, EditError> {
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
    pub(super) fn elevenlabs(&self) -> Result<&ApiKey, EditError> {
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
impl Default for Runtime {
    fn default() -> Self {
        Self {
            use_gpu: cfg!(target_os = "macos"),
            threads: 4,
            gipformer_runner: None,
        }
    }
}
impl ExecutionHooks for IdleHooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn stage(&self, _: ai::Stage) {}
}
impl TranscriberKind {
    pub(super) fn as_transcriber(&self) -> &dyn ai::Transcriber {
        match self {
            Self::Whisper(value) => value.as_ref(),
            Self::Gipformer(value) => value.as_ref(),
            Self::OpenAi(value) => value.as_ref(),
            Self::ElevenLabs(value) => value.as_ref(),
            Self::Other(value) => value.as_ref(),
        }
    }
    pub(super) fn whisper(&self) -> Option<&ai::whisper::Whisper> {
        match self {
            Self::Whisper(value) => Some(value.as_ref()),
            _ => None,
        }
    }
}
impl TranslatorKind {
    pub(super) fn as_translator(&self) -> &dyn ai::Translator {
        match self {
            Self::OpenAi(value) => value.as_ref(),
            Self::Anthropic(value) => value.as_ref(),
            Self::Other(value) => value.as_ref(),
        }
    }
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
    pub(super) fn transcriber_arc(&self) -> Arc<dyn ai::Transcriber> {
        match &self.transcriber {
            TranscriberKind::Whisper(value) => value.clone(),
            TranscriberKind::Gipformer(value) => value.clone(),
            TranscriberKind::OpenAi(value) => value.clone(),
            TranscriberKind::ElevenLabs(value) => value.clone(),
            TranscriberKind::Other(value) => value.clone(),
        }
    }
    pub(super) fn aligner_arc(&self) -> Option<Arc<dyn ai::Aligner>> {
        self.aligner.clone()
    }
    pub(crate) fn language_profile(
        &self,
        language: &LanguageTag,
    ) -> Result<LanguageProfile, EditError> {
        Ok(LanguageRegistry::standard()?.profile(language).clone())
    }
    pub(super) fn transcription_scope(
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
    pub(super) fn alignment_scope(
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
}

pub(super) fn translation_scope(
    translator: &dyn ai::Translator,
    mode: &NfcText,
    source: &LanguageTag,
    target: &LanguageTag,
    payload: &NfcText,
    fingerprint: &NfcText,
) -> Result<Scope, EditError> {
    Ok(Scope {
        provider: translator.config().clone(),
        operation: ai::Operation::Translate,
        payload_fingerprint: fingerprint.clone(),
        mode: mode.clone(),
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

pub(super) fn build_translator(
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
impl ConsentPrompt for DenyUploads {
    fn approve(&self, _: &UploadRequest) -> Result<(), EditError> {
        Err(EditError::ConsentRequired)
    }
}
impl ConsentPrompt for AllowAll {
    fn approve(&self, _: &UploadRequest) -> Result<(), EditError> {
        Ok(())
    }
}

pub(super) fn normalize_target(target: &LanguageTag) -> Result<LanguageTag, EditError> {
    if target.as_str() == "zh" {
        Ok(LanguageTag::parse("zh-Hans")?)
    } else {
        Ok(target.clone())
    }
}

impl Providers {
    pub(super) fn upload_request(&self, scope: Scope) -> Result<UploadRequest, EditError> {
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
}

impl TranslatorKind {
    pub(super) fn upload_request(&self, scope: Scope) -> Result<UploadRequest, EditError> {
        match self {
            Self::OpenAi(value) => {
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
            Self::Anthropic(value) => {
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
            Self::Other(value) => {
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
        }
    }
}

pub(crate) fn estimate_money(estimate: &CostEstimate) -> Result<Money, EditError> {
    match estimate {
        CostEstimate::Free => Ok(Money::new("USD", 0)?),
        CostEstimate::UpperBound(money) => Ok(money.clone()),
        CostEstimate::Unknown => Err(EditError::Ai(ai::Error::Cost)),
    }
}

pub(super) fn hex(bytes: &[u8]) -> String {
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
