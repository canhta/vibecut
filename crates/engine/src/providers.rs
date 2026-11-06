//! Resolves explicitly selected AI providers.
mod selection;
pub use selection::{
    Approval, ClipAttempt, ProviderOutput, TranscriptionAttempt, TranslationAttempt, suggest_clips,
    transcribe, translate,
};
mod model_transfers;
pub mod models {
    //! Transfers explicitly requested model bundles.
    pub(crate) use super::model_transfers::response;
    pub use super::model_transfers::{ModelIo, ModelTransfers, SystemModelIo};
}
