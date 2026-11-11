//! Failures of speech acquisition after provider selection.
use crate::runtime::Error as RuntimeError;
use crate::session::{
    EditError,
    wire::{ControlError, ErrorCode, Failure, JobFailurePolicy},
};

/// Acquisition failures retain their original lower execution and validation causes.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Edit(#[from] EditError),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error(transparent)]
    Providers(#[from] crate::providers::Error),
    #[error("media operation failed")]
    Media(#[from] media::MediaError),
    #[error("AI execution failed")]
    Ai(#[from] ai::Error),
    #[error("no speech recognized")]
    NoSpeech,
    #[error("speech recognition confidence is too low")]
    LowConfidence,
}
impl Failure for Error {
    fn transient(&self) -> bool {
        match self {
            Self::Runtime(error) => error.transient(),
            Self::Providers(error) => error.transient(),
            Self::Ai(error) => error.transient(),
            _ => false,
        }
    }
    fn job_policy(&self) -> JobFailurePolicy {
        match self {
            Self::Runtime(error) => error.job_policy(),
            Self::Providers(error) => error.job_policy(),
            Self::Ai(error) => error.job_policy(),
            _ => JobFailurePolicy::default(),
        }
    }
    /// The wire class of this refusal, stated where the variant is declared.
    fn code(&self) -> ErrorCode {
        match self {
            Self::Edit(error) => error.code(),
            Self::Runtime(error) => error.code(),
            Self::Providers(error) => error.code(),
            Self::Media(error) => error.code(),
            Self::Ai(error) => error.code(),
            Self::NoSpeech | Self::LowConfidence => ErrorCode::Refused,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
impl From<captions::Error> for Error {
    fn from(error: captions::Error) -> Self {
        Self::Edit(error.into())
    }
}
impl From<time::TimeError> for Error {
    fn from(error: time::TimeError) -> Self {
        Self::Edit(error.into())
    }
}
impl From<ControlError> for Error {
    fn from(error: ControlError) -> Self {
        Self::Edit(error.into())
    }
}
