//! Opens the chosen provider adapters with their model leases.
use super::catalog::{
    CatalogOperation, TextProvider, TranscriptionProvider, TranslationProvider, catalog_model,
};
use super::consent::UploadRequest;
use super::error::Error;
use crate::EditError;
use crate::cache::{ArtifactCache, ModelLease};
use ai::transport::{ApiKey, UreqTransport};
use ai::{ExecutionHooks, SampleRange, Scope};
use captions::LanguageTag;
use captions::{LanguageProfile, LanguageRegistry, NfcText};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
pub struct Credentials {
    openai: Option<String>,
    elevenlabs: Option<String>,
    anthropic: Option<String>,
}
/// Local execution choices. The runner is the shipped Gipformer helper, never a model.
#[derive(Clone, Debug)]
pub struct Runtime {
    pub use_gpu: bool,
    pub threads: u8,
    pub gipformer_runner: Option<PathBuf>,
}
pub(crate) struct IdleHooks;
/// Resolved adapter instances and the leases that keep native model files immutable.
pub struct Providers {
    pub(crate) transcriber: Arc<dyn ai::Transcriber>,
    pub(crate) aligner: Option<Arc<dyn ai::Aligner>>,
    pub(crate) translator: Option<Arc<dyn ai::Translator>>,
    pub(crate) transcription: TranscriptionProvider,
    pub(crate) translation: Option<TranslationProvider>,
    pub(crate) transcription_mode: NfcText,
    pub(crate) translation_mode: Option<NfcText>,
    pub(crate) needs_alignment: bool,
    _leases: Vec<ModelLease>,
}
impl Credentials {
    /// Capture private values without validating providers the operation may not use.
    /// Only the selected credential accessor validates; values never enter logs or errors.
    pub fn new(
        openai: Option<String>,
        elevenlabs: Option<String>,
        anthropic: Option<String>,
    ) -> Self {
        Self {
            openai,
            elevenlabs,
            anthropic,
        }
    }
    /// The composition roots' one source of secrets: the process environment.
    /// An unset variable is an absent credential; the selected adapter refuses it.
    pub fn from_env() -> Self {
        let var = |name| std::env::var(name).ok();
        Self::new(
            var("OPENAI_API_KEY"),
            var("ELEVENLABS_API_KEY"),
            var("ANTHROPIC_API_KEY"),
        )
    }
    /// Validate the selected OpenAI key, returning typed missing or invalid refusals.
    pub fn openai(&self) -> Result<ApiKey, EditError> {
        Ok(ApiKey::new(
            self.openai
                .as_ref()
                .ok_or(Error::CredentialMissing("OPENAI_API_KEY"))?
                .clone(),
        )?)
    }
    /// Validate the selected ElevenLabs key, returning typed missing or invalid refusals.
    pub fn elevenlabs(&self) -> Result<ApiKey, EditError> {
        Ok(ApiKey::new(
            self.elevenlabs
                .as_ref()
                .ok_or(Error::CredentialMissing("ELEVENLABS_API_KEY"))?
                .clone(),
        )?)
    }
    /// Validate the selected Anthropic key, returning typed missing or invalid refusals.
    pub fn anthropic(&self) -> Result<ApiKey, EditError> {
        Ok(ApiKey::new(
            self.anthropic
                .as_ref()
                .ok_or(Error::CredentialMissing("ANTHROPIC_API_KEY"))?
                .clone(),
        )?)
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
                    Arc::new(value) as Arc<dyn ai::Transcriber>,
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
                    Arc::new(value) as Arc<dyn ai::Transcriber>,
                    true,
                    "transcribe",
                )
            }
            TranscriptionProvider::OpenAi { model } => {
                let value = ai::openai::OpenAiTranscriber::new(
                    ai::openai::OpenAiTranscriber::provider_config(model)?,
                    credentials.openai()?,
                    Arc::new(UreqTransport::new()),
                )?;
                (
                    Arc::new(value) as Arc<dyn ai::Transcriber>,
                    true,
                    ai::openai::MODE,
                )
            }
            TranscriptionProvider::ElevenLabs { model } => {
                let value = ai::elevenlabs::ElevenLabsTranscriber::new(
                    ai::elevenlabs::ElevenLabsTranscriber::provider_config(model)?,
                    credentials.elevenlabs()?,
                    Arc::new(UreqTransport::new()),
                )?;
                (
                    Arc::new(value) as Arc<dyn ai::Transcriber>,
                    false,
                    ai::elevenlabs::MODE,
                )
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
                let (translator, mode) = build_translator(value, credentials)?;
                (Some(translator), Some(mode))
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
            transcriber,
            aligner,
            translator,
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
        self.transcriber.as_ref()
    }
    pub fn translator(&self) -> Option<&dyn ai::Translator> {
        self.translator.as_deref()
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
        payload: &NfcText,
    ) -> Result<Scope, EditError> {
        Ok(Scope::transcription(
            self.transcriber.config().clone(),
            self.transcription_mode.clone(),
            language.clone(),
            samples,
            media::SPEECH_SAMPLE_RATE,
            payload.clone(),
        )?)
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
        Ok(Scope::alignment(
            aligner.config().clone(),
            language.clone(),
            samples,
            media::SPEECH_SAMPLE_RATE,
            payload.clone(),
        ))
    }
}
pub(crate) fn build_translator(
    translation: &TranslationProvider,
    credentials: &Credentials,
) -> Result<(Arc<dyn ai::Translator>, NfcText), EditError> {
    match translation {
        TranslationProvider::OpenAi { model } => {
            let value = ai::openai::OpenAiTranslator::new(
                ai::openai::OpenAiTranslator::provider_config(model)?,
                credentials.openai()?,
                Arc::new(UreqTransport::new()),
            )?;
            Ok((Arc::new(value), ai::openai::TRANSLATION_MODE.into()))
        }
        TranslationProvider::Anthropic { model } => {
            let value = ai::anthropic::AnthropicTranslator::new(
                ai::anthropic::AnthropicTranslator::provider_config(model)?,
                credentials.anthropic()?,
                Arc::new(UreqTransport::new()),
            )?;
            Ok((Arc::new(value), ai::anthropic::TRANSLATION_MODE.into()))
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
            credentials.openai()?,
            Arc::new(UreqTransport::new()),
        )?),
        TextProvider::Anthropic { model } => {
            Arc::new(ai::chapters::AnthropicChapterGenerator::new(
                ai::chapters::AnthropicChapterGenerator::provider_config(model)?,
                credentials.anthropic()?,
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
            credentials.openai()?,
            Arc::new(UreqTransport::new()),
        )?),
        TextProvider::Anthropic { model } => Arc::new(ai::fillers::AnthropicFillerClassifier::new(
            ai::fillers::AnthropicFillerClassifier::provider_config(model)?,
            credentials.anthropic()?,
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
        UploadRequest::for_scope(
            CatalogOperation::Transcribe,
            self.transcriber.config(),
            self.transcriber.disclosure(&scope)?,
            &self.transcriber.estimate(&scope),
            &scope,
        )
    }
}
