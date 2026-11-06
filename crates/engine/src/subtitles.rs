//! Produces source-timed subtitle output from speech.
mod workflow;
pub use workflow::{
    AllowAll, Cancellation, CatalogOperation, ConsentPrompt, Cost, Credentials, DenyUploads,
    ModelDownloader, ModelFileInfo, ModelInfo, Progress, ProviderOption, Providers, Runtime,
    SilentProgress, Stage, SubtitleOutcome, SubtitleRequest, SubtitleTranslator,
    TRANSLATE_ATTEMPTS, Transcription, TranscriptionProvider, TranslationProvider,
    TranslationSelection, UploadRequest, choose_transcription, choose_translation,
    default_cache_root, local_asr_runner, model_info, model_infos, normalize_target_tag,
    parse_language, provider_options, provider_options_for, run, transcribe, translate_selected,
};
pub(crate) use workflow::{build_session, translate_track, wait_for_job};
mod ass_import;
pub use ass_import::{AssImportReport, AssImportRequest, prepare_ass_import};
mod translation_workflow;
pub mod translation {
    //! Translation selection, aggregate attempt budgets and explicit result application.
    pub use super::translation_workflow::{TranslationRun, TranslationTarget, window};
}
pub use captions::LanguageTag;
