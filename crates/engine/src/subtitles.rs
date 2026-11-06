//! Produces source-timed subtitles from explicitly selected providers.
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

/// Operation-wide attempt ceiling disclosed by desktop/CLI before translation.
pub const TRANSLATE_ATTEMPTS: u8 = 3;
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

/// One explicit model download. The command line is the affirmative action; the
/// downloader still requires the ai-owned consent token for the exact catalog record.
pub struct ModelDownloader {
    transfers: ModelTransfers,
}

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

struct IdleHooks;

enum TranscriberKind {
    Whisper(Arc<ai::whisper::Whisper>),
    Gipformer(Arc<ai::gipformer::Gipformer>),
    OpenAi(Arc<ai::openai::OpenAiTranscriber>),
    ElevenLabs(Arc<ai::elevenlabs::ElevenLabsTranscriber>),
    Other(Arc<dyn ai::Transcriber>),
}

enum TranslatorKind {
    OpenAi(Arc<ai::openai::OpenAiTranslator>),
    Anthropic(Arc<ai::anthropic::AnthropicTranslator>),
    Other(Arc<dyn ai::Translator>),
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

/// A provider disclosure shown before any remote byte leaves the machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cost {
    Free,
    UpperBound { currency: String, micros: u64 },
    Unknown,
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
pub struct AllowAll;

/// Operation-wide cooperative cancellation shared by extraction, every submitted
/// job and the later assembly/translation/export stages.
#[derive(Clone, Default)]
pub struct Cancellation {
    flag: Arc<AtomicBool>,
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

/// A translation-only adapter. Opening it never loads ASR weights or downloads files.
pub struct SubtitleTranslator {
    kind: TranslatorKind,
    mode: NfcText,
}
/// Exact cue selection and glossary bound into every remote request.
pub struct TranslationSelection {
    pub item: project::ItemId,
    pub ids: Vec<NfcText>,
    pub target: LanguageTag,
    pub glossary: Vec<ai::GlossaryTerm>,
    pub force: bool,
}
mod settings;

mod speech;

mod pipeline;

mod checkpoints;

mod persistence;

mod translation_application;

pub use settings::{
    choose_transcription, choose_translation, default_cache_root, local_asr_runner, model_info,
    model_infos, normalize_target_tag, parse_language, provider_options, provider_options_for,
};

use settings::{build_translator, hex, normalize_target, translation_scope};

pub(crate) use settings::estimate_money;

pub use speech::transcribe;

use speech::{media_duration, sample_range};

pub use pipeline::run;

use checkpoints::transcribe_window;

pub(crate) use checkpoints::wait_for_job;

pub(crate) use persistence::{build_session, caption_item_id};

use persistence::{
    burn_subtitles, caption_track_id, ensure_distinct, export_subtitles, resolve_path,
};

pub(crate) use translation_application::{translate_cues, translate_track};

pub use translation_application::translate_selected;

mod ass_import;
pub use ass_import::{AssImportReport, AssImportRequest, prepare_ass_import};
mod translation_workflow;
pub mod translation {
    //! Applies selected translations through bounded attempts.
    pub use super::translation_workflow::{TranslationRun, TranslationTarget, window};
}
