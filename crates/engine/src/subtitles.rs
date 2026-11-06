//! Produces source-timed subtitles from explicitly selected providers.
use std::{path::PathBuf, time::Duration};

pub use captions::LanguageTag;
// The providers and transcript owners are reachable here too, so the desktop and CLI keep
// their `engine::subtitles` paths until their imports move to the owners.
pub use crate::providers::{
    AllowAll, CatalogOperation, ConsentPrompt, Cost, Credentials, DenyUploads, ModelFileInfo,
    ModelInfo, ProviderOption, Providers, Runtime, TranscriptionProvider, TranslationProvider,
    UploadRequest, choose_transcription, choose_translation, default_cache_root, local_asr_runner,
    model_info, model_infos, normalize_target_tag, parse_language, provider_options,
    provider_options_for,
};
pub use crate::transcript::{
    Cancellation, Progress, SilentProgress, Stage, SubtitleTranslator, TRANSLATE_ATTEMPTS,
    Transcription, TranslationSelection, transcribe, translate_selected,
};

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

mod pipeline;

mod persistence;

pub use pipeline::run;

mod ass_import;
pub use ass_import::{AssImportReport, AssImportRequest, prepare_ass_import};
