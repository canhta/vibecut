//! Resolves the user's explicitly chosen AI providers.
mod adapters;
mod catalog;
mod chapters;
mod consent;
mod selection;
pub use adapters::{Credentials, Providers, Runtime};
pub(crate) use adapters::{IdleHooks, TranslatorKind, build_translator, translation_scope};
pub(crate) use catalog::normalize_target;
pub use catalog::{
    CatalogOperation, ChapterProvider, ModelFileInfo, ModelInfo, ProviderOption,
    TranscriptionProvider, TranslationProvider, catalog_model, choose_chapter,
    choose_transcription, choose_translation, default_cache_root, local_asr_runner, model_info,
    model_infos, normalize_target_tag, parse_language, provider_options, provider_options_for,
};
pub use chapters::{ChapterAccess, StoredConsent};
pub(crate) use consent::estimate_money;
pub use consent::{AllowAll, ConsentPrompt, Cost, DenyUploads, UploadRequest};
pub use selection::{
    Approval, ClipAttempt, ProviderOutput, TranscriptionAttempt, TranslationAttempt, suggest_clips,
    transcribe, translate,
};
mod model_transfers;
pub mod models {
    //! Transfers explicitly requested model bundles.
    pub use super::catalog_model;
    pub use super::model_transfers::{ModelIo, ModelTransfers, SystemModelIo};
    pub use ai::models::{DownloadConsent, Model};
}
