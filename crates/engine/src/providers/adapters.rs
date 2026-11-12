//! Opens the chosen provider adapters with their model leases.
use super::catalog::{
    CatalogOperation, TextProvider, TranscriptionProvider, TranslationProvider, catalog_model,
};
use super::consent::UploadRequest;
use super::error::{Error, Result};
use crate::cache::{ArtifactCache, ModelLease};
use ai::transport::ApiKey;
use ai::{ExecutionHooks, SampleRange, Scope};
use captions::LanguageTag;
use captions::{LanguageProfile, LanguageRegistry, NfcText};
use std::{path::PathBuf, sync::Arc};
use transport::UreqTransport;

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
/// The transcriber of a translation-only `Providers`: it never recognises speech.
struct NoTranscription {
    config: ai::ProviderConfig,
}
impl ai::Transcriber for NoTranscription {
    fn config(&self) -> &ai::ProviderConfig {
        &self.config
    }
    fn capabilities(&self, _: &Scope) -> ai::Capabilities {
        ai::Capabilities {
            model: self.config.model().clone(),
            mode: "none".into(),
            operation: ai::Operation::Transcribe,
            languages: Vec::new(),
            target_languages: Vec::new(),
            timing: std::collections::BTreeSet::new(),
            diarization: ai::Support::Unsupported,
            live_audio_input: ai::Support::Unsupported,
            file_result_stream: ai::Support::Unsupported,
            max_upload_bytes: None,
            max_duration_micros: None,
            evidence: ai::Evidence::Advertised,
            revision: "none".into(),
        }
    }
    fn estimate(&self, _: &Scope) -> ai::CostEstimate {
        ai::CostEstimate::Free
    }
    fn transcribe(
        &self,
        _: ai::AudioWindow<'_>,
        _: &ai::Request<'_>,
        _: &dyn ExecutionHooks,
    ) -> std::result::Result<Vec<ai::RecognitionSegment>, ai::Error> {
        Err(ai::Error::Unsupported)
    }
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
    pub fn openai(&self) -> Result<ApiKey> {
        Ok(ApiKey::new(
            self.openai
                .as_ref()
                .ok_or(Error::CredentialMissing("OPENAI_API_KEY"))?
                .clone(),
        )?)
    }
    /// Validate the selected ElevenLabs key, returning typed missing or invalid refusals.
    pub fn elevenlabs(&self) -> Result<ApiKey> {
        Ok(ApiKey::new(
            self.elevenlabs
                .as_ref()
                .ok_or(Error::CredentialMissing("ELEVENLABS_API_KEY"))?
                .clone(),
        )?)
    }
    /// Validate the selected Anthropic key, returning typed missing or invalid refusals.
    pub fn anthropic(&self) -> Result<ApiKey> {
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
    ) -> Result<Self> {
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
    ) -> Result<Self> {
        let (translator, mode) = build_translator(&translation, credentials)?;
        self.translator = Some(translator);
        self.translation = Some(translation);
        self.translation_mode = Some(mode);
        Ok(self)
    }

    /// Providers for a translation-only workflow: the selected translator and a transcriber
    /// that refuses every request, so translating needs no speech model or speech credential.
    pub fn translation_only(
        translation: TranslationProvider,
        credentials: &Credentials,
    ) -> Result<Self> {
        let (translator, mode) = build_translator(&translation, credentials)?;
        Self::translation_only_from_parts(translation, translator, mode)
    }

    /// `translation_only` over an already built translator and its resolved mode.
    pub fn translation_only_from_parts(
        translation: TranslationProvider,
        translator: Arc<dyn ai::Translator>,
        mode: NfcText,
    ) -> Result<Self> {
        let config = ai::ProviderConfig::new(
            "translation-only".into(),
            "none".into(),
            "none".into(),
            ai::Location::Local {
                backend: "none".into(),
                model_ref: "none".into(),
            },
        )?;
        Ok(Self {
            transcriber: Arc::new(NoTranscription { config }),
            aligner: None,
            translator: Some(translator),
            transcription: TranscriptionProvider::Whisper {
                model: "translation-only".into(),
            },
            translation: Some(translation),
            transcription_mode: "none".into(),
            translation_mode: Some(mode),
            needs_alignment: false,
            _leases: Vec::new(),
        })
    }

    /// Test/application composition with already built adapters and their resolved ASR mode.
    pub fn from_parts(
        transcription: TranscriptionProvider,
        transcription_mode: NfcText,
        transcriber: Arc<dyn ai::Transcriber>,
        aligner: Option<Arc<dyn ai::Aligner>>,
        translation: Option<TranslationProvider>,
        translator: Option<Arc<dyn ai::Translator>>,
    ) -> Result<Self> {
        let translation_mode = translation.as_ref().map(|_| "context-window".into());
        let needs_alignment = aligner.is_some();
        Ok(Self {
            transcriber,
            aligner,
            translator,
            transcription,
            translation,
            transcription_mode,
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
    pub(crate) fn language_profile(&self, language: &LanguageTag) -> Result<LanguageProfile> {
        Ok(LanguageRegistry::standard()?.profile(language).clone())
    }
    pub(crate) fn transcription_scope(
        &self,
        language: &LanguageTag,
        samples: SampleRange,
        payload: &NfcText,
    ) -> Result<Scope> {
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
    ) -> Result<Scope> {
        let aligner = self
            .aligner
            .as_ref()
            .ok_or(Error::Edit(crate::EditError::Unsupported(
                "missing aligner",
            )))?;
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
) -> Result<(Arc<dyn ai::Translator>, NfcText)> {
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
) -> Result<Arc<dyn ai::chapters::ChapterGenerator>> {
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
) -> Result<Arc<dyn ai::fillers::FillerClassifier>> {
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
/// Build the selected emphasis selector; it shares the approved text model, provider
/// configuration and credential with chapters and fillers.
pub(crate) fn open_emphasis(
    emphasis: &TextProvider,
    credentials: &Credentials,
) -> Result<Arc<dyn ai::emphasis::EmphasisSelector>> {
    Ok(match emphasis {
        TextProvider::OpenAi { model } => Arc::new(ai::emphasis::OpenAiEmphasisSelector::new(
            ai::emphasis::OpenAiEmphasisSelector::provider_config(model)?,
            credentials.openai()?,
            Arc::new(UreqTransport::new()),
        )?),
        TextProvider::Anthropic { model } => {
            Arc::new(ai::emphasis::AnthropicEmphasisSelector::new(
                ai::emphasis::AnthropicEmphasisSelector::provider_config(model)?,
                credentials.anthropic()?,
                Arc::new(UreqTransport::new()),
            )?)
        }
    })
}
fn load_aligner(
    cache: &mut ArtifactCache,
    leases: &mut Vec<ModelLease>,
) -> Result<Arc<dyn ai::Aligner>> {
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
        Err(Error::Edit(crate::EditError::Unsupported(
            "local alignment support is not enabled in this build",
        )))
    }
}
impl Providers {
    pub(crate) fn upload_request(&self, scope: Scope) -> Result<UploadRequest> {
        UploadRequest::for_scope(
            CatalogOperation::Transcribe,
            self.transcriber.config(),
            self.transcriber.disclosure(&scope)?,
            &self.transcriber.estimate(&scope),
            &scope,
        )
    }
}
