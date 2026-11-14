//! Offline provider policy and validated transcription/alignment/translation execution.
pub mod foundation;
pub mod providers;
pub mod recognition;
pub mod speech;
pub mod text;
pub mod vision;

use foundation::{contracts, remote};
use recognition::assembly;
use text::translation;

pub use assembly::*;
pub use contracts::*;
pub use remote::{MAX_REMOTE_MODELS, UploadDisclosure, defaults, list_remote_models};
pub use translation::*;

/// Transcript-text operations accept these exact BCP 47 tags; `zh` is a request
/// tag that adapters normalise rather than a distinct model language.
pub const TEXT_LANGUAGES: [&str; 5] = ["vi", "en", "zh", "zh-Hans", "zh-Hant"];
/// Whether a transcript-text operation accepts this language.
pub fn supports_text_language(language: &captions::LanguageTag) -> bool {
    TEXT_LANGUAGES.contains(&language.as_str())
}

/// Safe categories only: adapters must not attach response bodies or credentials.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("selected remote model is unavailable: {message}")]
    ModelUnavailable {
        provider: captions::NfcText,
        model: captions::NfcText,
        /// Bounded provider-error category, never a raw response body or message.
        message: &'static str,
    },
    #[error("alignment attention is unavailable or degenerate")]
    AttentionUnavailable,
    #[error("structured output failed schema or semantic validation")]
    Schema,
    #[error("provider refused the operation")]
    Refused,
    #[error("provider response is incomplete")]
    Incomplete,
    #[error("invalid AI input: {0}")]
    Invalid(&'static str),
    #[error("unsupported AI capability")]
    Unsupported,
    #[error("unsupported voice")]
    UnsupportedVoice,
    #[error("speech output rejected")]
    OutputRejected,
    #[error("provider content policy refused the request")]
    ContentPolicy,
    #[error("provider request timed out")]
    Timeout,
    #[error("unsupported language")]
    UnsupportedLanguage,
    #[error("timing unavailable")]
    TimingUnavailable,
    #[error("authentication failed")]
    Auth,
    #[error("quota exhausted")]
    Quota,
    /// The provider keeps this voice behind a paid plan; a different voice is the remedy.
    #[error("voice requires a paid plan")]
    PaidPlanRequired,
    #[error("rate limited; retry after {retry_after_seconds:?} seconds")]
    RateLimit { retry_after_seconds: Option<u32> },
    #[error("network request failed")]
    Network,
    #[error("provider server failed")]
    Server,
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
    #[error("vision tensor shape or bounded input is invalid")]
    VisionShape,
    #[error("vision model cannot be loaded or specialised")]
    VisionLoad,
    #[error("vision tensor contains non-finite or degenerate values")]
    VisionNonFinite,
    #[error("local inference failed")]
    Inference,
    #[error("AI timing conversion failed")]
    Time(#[from] time::TimeError),
    #[error("invalid caption output")]
    Captions(#[from] captions::Error),
    #[error("chapter generation refused: {0}")]
    Chapters(#[from] text::chapters::ChapterRefusal),
    #[error("filler classification refused: {0}")]
    Fillers(#[from] text::fillers::FillerRefusal),
    #[error("caption emphasis refused: {0}")]
    Emphasis(#[from] text::emphasis::EmphasisRefusal),
}

impl Error {
    /// Conditions the engine job runner may retry with its own bounded budget.
    /// Adapters never retry a request themselves.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::Network | Self::Timeout | Self::Server | Self::RateLimit { .. } | Self::Schema
        )
    }
}

impl From<::transport::Error> for Error {
    fn from(error: ::transport::Error) -> Self {
        match error {
            ::transport::Error::Invalid(message) => Self::Invalid(message),
            ::transport::Error::Payload => Self::Payload,
            ::transport::Error::Network => Self::Network,
        }
    }
}
