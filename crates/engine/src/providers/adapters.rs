//! Opens the chosen provider adapters with their model leases.
use super::catalog::{
    CatalogOperation, TextProvider, TranscriptionProvider, TranslationProvider, catalog_model,
};
use super::consent::{Cost, UploadRequest};
use super::error::Error;
use crate::EditError;
use crate::cache::{ArtifactCache, ModelLease};
use ai::transport::{ApiKey, UreqTransport};
use ai::{ExecutionHooks, SampleRange, Scope, Transcriber, Translator};
use captions::LanguageTag;
use captions::{LanguageProfile, LanguageRegistry, NfcText};
use std::{path::PathBuf, sync::Arc};
const MAX_JSON_BYTES: usize = 64 * 1024;

#[derive(Clone)]
pub struct Credentials {
    openai: Option<ApiKey>,
    elevenlabs: Option<ApiKey>,
    anthropic: Option<ApiKey>,
}
/// Local execution choices. The runner is the shipped Gipformer helper, never a model.
#[derive(Clone, Debug)]
pub struct Runtime {
    pub use_gpu: bool,
    pub threads: u8,
    pub gipformer_runner: Option<PathBuf>,
}
pub(crate) struct IdleHooks;
pub(crate) enum TranscriberKind {
    Whisper(Arc<ai::whisper::Whisper>),
    Gipformer(Arc<ai::gipformer::Gipformer>),
    OpenAi(Arc<ai::openai::OpenAiTranscriber>),
    ElevenLabs(Arc<ai::elevenlabs::ElevenLabsTranscriber>),
    Other(Arc<dyn ai::Transcriber>),
}
pub(crate) enum TranslatorKind {
    OpenAi(Arc<ai::openai::OpenAiTranslator>),
    Anthropic(Arc<ai::anthropic::AnthropicTranslator>),
    Other(Arc<dyn ai::Translator>),
}
/// Resolved adapter instances and the leases that keep native model files immutable.
pub struct Providers {
    pub(crate) transcriber: TranscriberKind,
    pub(crate) aligner: Option<Arc<dyn ai::Aligner>>,
    pub(crate) translator: Option<TranslatorKind>,
    pub(crate) transcription: TranscriptionProvider,
    pub(crate) translation: Option<TranslationProvider>,
    pub(crate) transcription_mode: NfcText,
    pub(crate) translation_mode: Option<NfcText>,
    pub(crate) needs_alignment: bool,
    _leases: Vec<ModelLease>,
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
    /// Resolve the configured OpenAI credential for a selected adapter or refuse.
    pub fn openai(&self) -> Result<&ApiKey, EditError> {
        Ok(self
            .openai
            .as_ref()
            .ok_or(Error::CredentialMissing("OPENAI_API_KEY"))?)
    }
    pub(crate) fn elevenlabs(&self) -> Result<&ApiKey, EditError> {
        Ok(self
            .elevenlabs
            .as_ref()
            .ok_or(Error::CredentialMissing("ELEVENLABS_API_KEY"))?)
    }
    /// Resolve the configured Anthropic credential for a selected adapter or refuse.
    pub fn anthropic(&self) -> Result<&ApiKey, EditError> {
        Ok(self
            .anthropic
            .as_ref()
            .ok_or(Error::CredentialMissing("ANTHROPIC_API_KEY"))?)
    }
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
    pub(crate) fn as_transcriber(&self) -> &dyn ai::Transcriber {
        match self {
            Self::Whisper(value) => value.as_ref(),
            Self::Gipformer(value) => value.as_ref(),
            Self::OpenAi(value) => value.as_ref(),
            Self::ElevenLabs(value) => value.as_ref(),
            Self::Other(value) => value.as_ref(),
        }
    }
    pub(crate) fn whisper(&self) -> Option<&ai::whisper::Whisper> {
        match self {
            Self::Whisper(value) => Some(value.as_ref()),
            _ => None,
        }
    }
}
impl TranslatorKind {
    pub(crate) fn as_translator(&self) -> &dyn ai::Translator {
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
                let metadata = catalog_model(model)?;
                let lease = cache
                    .model(&metadata, &IdleHooks)?
                    .ok_or(Error::ModelMissing("whisper-large-v3-turbo"))?;
                let path = lease.file(
                    &metadata
                        .files
                        .first()
                        .ok_or(Error::ModelMissing("whisper-large-v3-turbo"))?
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
                let metadata = catalog_model(model)?;
                let lease = cache
                    .model(&metadata, &IdleHooks)?
                    .ok_or(Error::ModelMissing("gipformer-68m-rnnt-int8"))?;
                let runner = runtime
                    .gipformer_runner
                    .as_deref()
                    .ok_or(Error::RunnerMissing)?;
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
                let value = ai::openai::OpenAiTranscriber::new(
                    ai::openai::OpenAiTranscriber::provider_config(model)?,
                    credentials.openai()?.clone(),
                    Arc::new(UreqTransport::new()),
                )?;
                (TranscriberKind::OpenAi(Arc::new(value)), true, "file")
            }
            TranscriptionProvider::ElevenLabs { model } => {
                let value = ai::elevenlabs::ElevenLabsTranscriber::new(
                    ai::elevenlabs::ElevenLabsTranscriber::provider_config(model)?,
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
    pub(crate) fn transcriber_arc(&self) -> Arc<dyn ai::Transcriber> {
        match &self.transcriber {
            TranscriberKind::Whisper(value) => value.clone(),
            TranscriberKind::Gipformer(value) => value.clone(),
            TranscriberKind::OpenAi(value) => value.clone(),
            TranscriberKind::ElevenLabs(value) => value.clone(),
            TranscriberKind::Other(value) => value.clone(),
        }
    }
    pub(crate) fn aligner_arc(&self) -> Option<Arc<dyn ai::Aligner>> {
        self.aligner.clone()
    }
    pub(crate) fn language_profile(
        &self,
        language: &LanguageTag,
    ) -> Result<LanguageProfile, EditError> {
        Ok(LanguageRegistry::standard()?.profile(language).clone())
    }
    pub(crate) fn transcription_scope(
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
    pub(crate) fn alignment_scope(
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
pub(crate) fn translation_scope(
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
pub(crate) fn build_translator(
    translation: &TranslationProvider,
    credentials: &Credentials,
) -> Result<(TranslatorKind, NfcText), EditError> {
    match translation {
        TranslationProvider::OpenAi { model } => {
            let value = ai::openai::OpenAiTranslator::new(
                ai::openai::OpenAiTranslator::provider_config(model)?,
                credentials.openai()?.clone(),
                Arc::new(UreqTransport::new()),
            )?;
            Ok((
                TranslatorKind::OpenAi(Arc::new(value)),
                ai::openai::TRANSLATION_MODE.into(),
            ))
        }
        TranslationProvider::Anthropic { model } => {
            let value = ai::anthropic::AnthropicTranslator::new(
                ai::anthropic::AnthropicTranslator::provider_config(model)?,
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
/// Build the selected chapter adapter; the exact configuration is what the
/// chapter consent scope binds.
pub(crate) fn open_chapter(
    chapter: &TextProvider,
    credentials: &Credentials,
) -> Result<Arc<dyn ai::chapters::ChapterGenerator>, EditError> {
    Ok(match chapter {
        TextProvider::OpenAi { model } => Arc::new(ai::chapters::OpenAiChapterGenerator::new(
            ai::chapters::OpenAiChapterGenerator::provider_config(model)?,
            credentials.openai()?.clone(),
            Arc::new(UreqTransport::new()),
        )?),
        TextProvider::Anthropic { model } => {
            Arc::new(ai::chapters::AnthropicChapterGenerator::new(
                ai::chapters::AnthropicChapterGenerator::provider_config(model)?,
                credentials.anthropic()?.clone(),
                Arc::new(UreqTransport::new()),
            )?)
        }
    })
}
/// Build the selected filler classifier; it shares the approved text model,
/// provider configuration and credential with chapter generation.
pub(crate) fn open_filler(
    filler: &TextProvider,
    credentials: &Credentials,
) -> Result<Arc<dyn ai::fillers::FillerClassifier>, EditError> {
    Ok(match filler {
        TextProvider::OpenAi { model } => Arc::new(ai::fillers::OpenAiFillerClassifier::new(
            ai::fillers::OpenAiFillerClassifier::provider_config(model)?,
            credentials.openai()?.clone(),
            Arc::new(UreqTransport::new()),
        )?),
        TextProvider::Anthropic { model } => Arc::new(ai::fillers::AnthropicFillerClassifier::new(
            ai::fillers::AnthropicFillerClassifier::provider_config(model)?,
            credentials.anthropic()?.clone(),
            Arc::new(UreqTransport::new()),
        )?),
    })
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
            .ok_or(Error::ModelMissing("whisper-small-dtw"))?;
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
impl Providers {
    pub(crate) fn upload_request(&self, scope: Scope) -> Result<UploadRequest, EditError> {
        match &self.transcriber {
            TranscriberKind::OpenAi(value) => {
                transcriber_upload(value.disclosure(&scope)?, CatalogOperation::Transcribe)
            }
            TranscriberKind::ElevenLabs(value) => {
                transcriber_upload(value.disclosure(&scope)?, CatalogOperation::Transcribe)
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

fn transcriber_upload(
    disclosure: Option<ai::UploadDisclosure>,
    operation: CatalogOperation,
) -> Result<UploadRequest, EditError> {
    let disclosure = disclosure.ok_or(EditError::Unsupported("provider has no upload"))?;
    Ok(UploadRequest {
        operation,
        provider: disclosure.provider.as_str().into(),
        model: disclosure.model.as_str().into(),
        languages: disclosure.languages.as_str().into(),
        audio_seconds: disclosure.audio_micros as f64 / 1_000_000.0,
        upload_bytes: disclosure.upload_bytes,
        cost: Cost::from(&disclosure.estimate),
        terms: disclosure
            .links
            .iter()
            .map(|(name, url)| (*name, url.as_str().into()))
            .collect(),
    })
}
impl TranslatorKind {
    pub(crate) fn upload_request(&self, scope: Scope) -> Result<UploadRequest, EditError> {
        match self {
            Self::OpenAi(value) => translator_upload(value.disclosure(&scope)?, &scope),
            Self::Anthropic(value) => translator_upload(value.disclosure(&scope)?, &scope),
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

fn translator_upload(
    disclosure: Option<ai::UploadDisclosure>,
    scope: &Scope,
) -> Result<UploadRequest, EditError> {
    let disclosure = disclosure.ok_or(EditError::Unsupported("provider has no upload"))?;
    Ok(UploadRequest {
        operation: CatalogOperation::Translate,
        provider: disclosure.provider.as_str().into(),
        model: disclosure.model.as_str().into(),
        languages: disclosure.languages.as_str().into(),
        audio_seconds: 0.0,
        upload_bytes: u64::try_from(scope.texts.first().map_or(0, |t| t.as_str().len()))
            .map_err(|_| EditError::Limit)?,
        cost: Cost::from(&disclosure.estimate),
        terms: disclosure
            .links
            .iter()
            .map(|(name, url)| (*name, url.as_str().into()))
            .collect(),
    })
}
