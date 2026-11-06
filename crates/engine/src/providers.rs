//! Resolves explicitly selected AI providers.
mod selection;
pub use selection::{
    Approval, ClipAttempt, ProviderOutput, TranscriptionAttempt, TranslationAttempt, suggest_clips,
    transcribe, translate,
};
mod model_transfers;
pub mod models {
    //! Explicitly consented model transfers, through the shared runner/cache/transport.
    pub(crate) use super::model_transfers::response;
    pub use super::model_transfers::{ModelIo, ModelTransfers, SystemModelIo};
}
