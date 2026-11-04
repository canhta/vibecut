//! Offline provider policy and validated transcription/alignment/translation execution.
mod assembly;
mod contracts;
pub mod models;
mod policy;
#[cfg(feature = "whisper")]
pub mod whisper;
pub use assembly::*;
pub use contracts::*;
pub use policy::*;

/// Safe categories only: adapters must not attach response bodies or credentials.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid AI input: {0}")]
    Invalid(&'static str),
    #[error("unsupported AI capability")]
    Unsupported,
    #[error("unsupported language")]
    UnsupportedLanguage,
    #[error("timing unavailable")]
    TimingUnavailable,
    #[error("authentication failed")]
    Auth,
    #[error("quota exhausted")]
    Quota,
    #[error("rate limited; retry after {retry_after_seconds:?} seconds")]
    RateLimit { retry_after_seconds: Option<u32> },
    #[error("payload exceeds capability limits")]
    Payload,
    #[error("explicit consent required for this scope")]
    ConsentRequired,
    #[error("cost exceeds authorized budget or cannot be estimated")]
    Cost,
    #[error("operation cancelled")]
    Cancelled,
    #[error("model file failed size or SHA-256 verification")]
    ModelIntegrity,
    #[error("model storage is full")]
    StorageFull,
    #[error("model storage failed: {0:?}")]
    ModelStorage(std::io::ErrorKind),
    #[error("local inference failed")]
    Inference,
    #[error("AI timing conversion failed")]
    Time(#[from] time::TimeError),
    #[error("invalid caption output")]
    Captions(#[from] captions::Error),
}
