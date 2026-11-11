//! Failures owned by provider selection, local adapter setup and upload consent.
use crate::runtime::Error as RuntimeError;
use crate::session::{
    EditError,
    wire::{ErrorCode, Failure, JobFailurePolicy},
};

/// A provider selection, local adapter setup or consent refusal.
///
/// Display carries no command to run: composition roots present their own hint.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Edit(#[from] EditError),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error("AI execution failed")]
    Ai(#[from] ai::Error),
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
impl Failure for Error {
    fn transient(&self) -> bool {
        match self {
            Self::Runtime(error) => error.transient(),
            Self::Ai(error) => error.transient(),
            _ => false,
        }
    }
    fn job_policy(&self) -> JobFailurePolicy {
        match self {
            Self::Runtime(error) => error.job_policy(),
            Self::Ai(error) => error.job_policy(),
            _ => JobFailurePolicy::default(),
        }
    }
    /// The wire class of this refusal, stated where the variant is declared.
    fn code(&self) -> ErrorCode {
        match self {
            Self::Edit(error) => error.code(),
            Self::Runtime(error) => error.code(),
            Self::Ai(error) => error.code(),
            Self::ConsentRequired => ErrorCode::Permission,
            Self::AutoLanguageUnsupported => ErrorCode::Refused,
            Self::ModelMissing(_) | Self::RunnerMissing | Self::CredentialMissing(_) => {
                ErrorCode::Setup
            }
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
impl From<captions::Error> for Error {
    fn from(error: captions::Error) -> Self {
        Self::Edit(error.into())
    }
}
