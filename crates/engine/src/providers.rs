//! Resolves the user's explicitly chosen AI providers.
mod adapters;
mod catalog;
mod consent;
mod error;
mod text;
pub(crate) use adapters::build_translator;
pub use adapters::{Credentials, Providers, Runtime};
pub(crate) use catalog::normalize_target;
pub use catalog::{
    CatalogOperation, ModelFileInfo, ModelInfo, ProviderOption, TextProvider,
    TranscriptionProvider, TranslationProvider, catalog_model, choose_chapter, choose_filler,
    choose_transcription, choose_translation, local_asr_runner, model_info, model_infos,
    normalize_target_tag, parse_language, provider_options, provider_options_for,
};
pub(crate) use consent::estimate_money;
pub use consent::{AllowAll, ConsentPrompt, Cost, DenyUploads, UploadRequest, approve_upload};
pub use error::Error;
pub use text::{StoredConsent, TextAccess};
mod model_transfers;
pub mod models {
    //! Transfers explicitly requested model bundles.
    pub use super::catalog_model;
    pub use super::model_transfers::{Error, ModelIo, ModelTransfers, Result, SystemModelIo};
    pub use ai::models::{DownloadConsent, Model};
}
