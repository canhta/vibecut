//! Finite failure records shared by `EditError` and the external-asset features.

use super::wire::{ErrorCode, Failure};

/// Refusals owned by Session identity, command lifetime and history custody.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("identifier space exhausted")]
    IdExhausted,
    #[error("a gesture or transaction is active")]
    TransactionActive,
    #[error("no gesture is active")]
    NoGesture,
    #[error("no history step available")]
    NoHistory,
    #[error("command unwound; transaction rolled back")]
    Panicked,
    #[error("control identity unavailable")]
    ControlIdentity,
    #[error("job result is stale")]
    StaleResult,
    #[error("fork replay did not reproduce the fork")]
    ForkDiverged,
}
impl Failure for SessionError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::TransactionActive | Self::NoGesture => ErrorCode::TransactionActive,
            Self::NoHistory | Self::IdExhausted => ErrorCode::Refused,
            Self::Panicked | Self::ForkDiverged => ErrorCode::Failed,
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

/// Brand kit store failures.
#[derive(Debug, thiserror::Error)]
pub enum BrandKitError {
    #[error("the brand kit store is not available in this session")]
    Unavailable,
    #[error("no brand kit or entry has this identity")]
    NotFound,
    #[error("brand kit limit reached")]
    Limit,
    #[error("brand kit input is invalid: {0}")]
    Invalid(&'static str),
    #[error("brand kit entry is missing or fails verification")]
    Integrity,
    #[error("brand kit IO failed")]
    Io(#[from] std::io::Error),
    #[error("brand kit persistence failed")]
    Project(#[from] project::Error),
}
impl Failure for BrandKitError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Unavailable => ErrorCode::Unavailable,
            Self::NotFound | Self::Integrity => ErrorCode::Refused,
            Self::Limit => ErrorCode::Limit,
            Self::Invalid(_) => ErrorCode::InvalidParams,
            Self::Io(_) | Self::Project(_) => ErrorCode::Io,
        }
    }
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
impl From<transport::Error> for SoundError {
    fn from(error: transport::Error) -> Self {
        match error {
            transport::Error::Network => Self::Network,
            transport::Error::Payload => Self::Limit,
            transport::Error::Invalid(_) => Self::InvalidResponse,
        }
    }
}

/// Music library failures.
#[derive(Debug, thiserror::Error)]
pub enum MusicError {
    #[error("music source credential was rejected")]
    Auth,
    #[error("music source quota is exhausted")]
    Quota,
    #[error("music source rate limit reached; retry after {retry_after_seconds:?} seconds")]
    RateLimit { retry_after_seconds: Option<u32> },
    #[error("music source network request failed")]
    Network,
    #[error("music source server failed")]
    Server,
    #[error("music source response is invalid")]
    InvalidResponse,
    #[error("music response exceeds the configured limit")]
    Limit,
    #[error("music audio device is unavailable")]
    NoAudioDevice,
    #[error("music audio is not cached")]
    NotCached,
    #[error("music library file is corrupt or unsupported")]
    LibraryCorrupt,
    #[error("music operation cancelled")]
    Cancelled,
    #[error("music IO failed")]
    Io(#[from] std::io::Error),
    #[error("music media failed")]
    Media(#[from] media::MediaError),
    #[error("music library persistence failed")]
    Project(#[from] project::Error),
    #[error("invalid music input: {0}")]
    Invalid(&'static str),
}
impl From<transport::Error> for MusicError {
    fn from(error: transport::Error) -> Self {
        match error {
            transport::Error::Network => Self::Network,
            transport::Error::Payload => Self::Limit,
            transport::Error::Invalid(_) => Self::InvalidResponse,
        }
    }
}
impl From<SoundError> for MusicError {
    fn from(error: SoundError) -> Self {
        match error {
            SoundError::NoCredentials | SoundError::Auth => Self::Auth,
            SoundError::Quota => Self::Quota,
            SoundError::RateLimit {
                retry_after_seconds,
            } => Self::RateLimit {
                retry_after_seconds,
            },
            SoundError::Network => Self::Network,
            SoundError::Server => Self::Server,
            SoundError::Limit => Self::Limit,
            SoundError::NoAudioDevice => Self::NoAudioDevice,
            SoundError::NotCached => Self::NotCached,
            SoundError::LibraryCorrupt => Self::LibraryCorrupt,
            SoundError::Cancelled => Self::Cancelled,
            SoundError::Io(error) => Self::Io(error),
            SoundError::Media(error) => Self::Media(error),
            SoundError::Project(error) => Self::Project(error),
            SoundError::Invalid(_) | SoundError::InvalidResponse => Self::InvalidResponse,
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
impl Failure for MusicError {
    fn code(&self) -> ErrorCode {
        match self {
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
