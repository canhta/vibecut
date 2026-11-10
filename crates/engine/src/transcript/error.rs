//! Failures of speech acquisition after provider selection.
use crate::session::wire::ErrorCode;

/// A recognition result that refuses to insert cues.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no speech recognized")]
    NoSpeech,
    #[error("speech recognition confidence is too low")]
    LowConfidence,
}
impl Error {
    /// The wire class of this refusal, stated where the variant is declared.
    pub(crate) fn code(&self) -> ErrorCode {
        match self {
            Self::NoSpeech | Self::LowConfidence => ErrorCode::Refused,
        }
    }
}
