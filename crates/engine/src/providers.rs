//! Resolves explicitly selected AI providers.
mod selection;
pub use selection::{
    Approval, ClipAttempt, ProviderOutput, TranscriptionAttempt, TranslationAttempt, suggest_clips,
    transcribe, translate,
};
mod model_transfers;
pub mod models {
    //! Transfers explicitly requested model bundles.
    pub use super::model_transfers::{ModelIo, ModelTransfers, SystemModelIo};
}
