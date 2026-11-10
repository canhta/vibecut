//! Failures owned by provider selection, local adapter setup and upload consent.
use crate::session::wire::ErrorCode;

/// A provider selection, local adapter setup or consent refusal.
///
/// Display carries no command to run: composition roots present their own hint.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("local model {0} is not installed")]
    ModelMissing(&'static str),
    #[error("local Gipformer runner is not installed")]
    RunnerMissing,
    #[error("provider {0} requires its API key")]
    CredentialMissing(&'static str),
    #[error("auto language detection requires the local whisper.cpp provider")]
    AutoLanguageUnsupported,
    #[error("remote upload requires explicit consent")]
    ConsentRequired,
}
impl Error {
    /// The wire class of this refusal, stated where the variant is declared.
    pub(crate) fn code(&self) -> ErrorCode {
        match self {
            Self::ConsentRequired => ErrorCode::Permission,
            Self::ModelMissing(_)
            | Self::RunnerMissing
            | Self::CredentialMissing(_)
            | Self::AutoLanguageUnsupported => ErrorCode::Refused,
        }
    }
}
