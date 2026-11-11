//! Finite failure records shared by `EditError` and the external-asset features.

use super::wire::{ErrorCode, Failure};

/// Refusals owned by Session identity and revision custody.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("control identity unavailable")]
    ControlIdentity,
    #[error("job result is stale")]
    StaleResult,
}
impl Failure for SessionError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::StaleResult => ErrorCode::StaleResult,
            Self::ControlIdentity => ErrorCode::Refused,
        }
    }
}

/// Font catalog failures that are not a load state.
#[derive(Debug, thiserror::Error)]
pub enum FontError {
    #[error("the font store is not available in this session")]
    Unavailable,
    #[error("family is not a downloadable catalog font")]
    NotDownloadable,
    #[error("a download for this family is already running")]
    DownloadActive,
    #[error("stored font bytes or metadata failed verification")]
    Integrity,
    #[error("font server did not answer with a complete file")]
    Response,
    #[error("font file cannot be registered")]
    Face(#[from] text::FontError),
    #[error("font catalog entry is invalid")]
    Catalog,
    #[error("font input is corrupt or unsupported")]
    InvalidImport,
    #[error("different font bytes already own this family and style")]
    Conflict,
    #[error("a font import is already running")]
    ImportActive,
}

/// Sound library failures.
#[derive(Debug, thiserror::Error)]
pub enum SoundError {
    #[error("Freesound credential is absent")]
    NoCredentials,
    #[error("Freesound authentication failed")]
    Auth,
    #[error("Freesound quota is exhausted")]
    Quota,
    #[error("Freesound rate limit reached; retry after {retry_after_seconds:?} seconds")]
    RateLimit { retry_after_seconds: Option<u32> },
    #[error("Freesound network request failed")]
    Network,
    #[error("Freesound server failed")]
    Server,
    #[error("Freesound response is invalid")]
    InvalidResponse,
    #[error("sound response exceeds the configured limit")]
    Limit,
    #[error("sound audio device is unavailable")]
    NoAudioDevice,
    #[error("sound audio is not cached")]
    NotCached,
    #[error("sound library file is corrupt or unsupported")]
    LibraryCorrupt,
    #[error("sound operation cancelled")]
    Cancelled,
    #[error("sound IO failed")]
    Io(#[from] std::io::Error),
    #[error("sound media failed")]
    Media(#[from] media::MediaError),
    #[error("sound library persistence failed")]
    Project(#[from] project::Error),
    #[error("invalid sound input: {0}")]
    Invalid(&'static str),
}
impl From<ai::Error> for SoundError {
    fn from(error: ai::Error) -> Self {
        match error {
            ai::Error::Auth => Self::Auth,
            ai::Error::Quota => Self::Quota,
            ai::Error::RateLimit {
                retry_after_seconds,
            } => Self::RateLimit {
                retry_after_seconds,
            },
            ai::Error::Network | ai::Error::Timeout => Self::Network,
            ai::Error::Server => Self::Server,
            ai::Error::Payload => Self::Limit,
            ai::Error::Cancelled => Self::Cancelled,
            _ => Self::InvalidResponse,
        }
    }
}

impl Failure for FontError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Unavailable
            | Self::NotDownloadable
            | Self::DownloadActive
            | Self::Integrity
            | Self::Response
            | Self::Face(_)
            | Self::Catalog
            | Self::InvalidImport
            | Self::Conflict
            | Self::ImportActive => ErrorCode::Refused,
        }
    }
}
impl Failure for SoundError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::NoCredentials => ErrorCode::Setup,
            Self::Auth => ErrorCode::Auth,
            Self::Quota => ErrorCode::Quota,
            Self::RateLimit { .. } => ErrorCode::RateLimit,
            Self::Network | Self::Server => ErrorCode::Network,
            Self::NoAudioDevice => ErrorCode::AudioDevice,
            Self::Cancelled => ErrorCode::SoundCancelled,
            Self::Limit => ErrorCode::SoundLimit,
            Self::InvalidResponse | Self::NotCached | Self::LibraryCorrupt | Self::Invalid(_) => {
                ErrorCode::Sounds
            }
            Self::Io(_) | Self::Project(_) => ErrorCode::SoundIo,
            Self::Media(_) => ErrorCode::Media,
        }
    }
}
