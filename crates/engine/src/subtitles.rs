//! Produces source-timed subtitles from explicitly selected providers.
use std::{path::PathBuf, time::Duration};

pub use captions::LanguageTag;

use crate::providers::{Cost, TranscriptionProvider, TranslationProvider};

/// Subtitle workflows retain their actual validation, resource and execution causes.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Edit(#[from] crate::session::EditError),
    #[error(transparent)]
    Runtime(#[from] crate::runtime::Error),
    #[error(transparent)]
    Transcript(#[from] crate::transcript::Error),
    #[error(transparent)]
    Providers(#[from] crate::providers::Error),
    #[error("media operation failed")]
    Media(#[from] media::MediaError),
    #[error("AI execution failed")]
    Ai(#[from] ai::Error),
    #[error("job/cache IO failed")]
    Io(#[from] std::io::Error),
}
pub type Result<T> = std::result::Result<T, Error>;

impl crate::session::wire::Failure for Error {
    fn code(&self) -> crate::session::wire::ErrorCode {
        match self {
            Self::Edit(error) => error.code(),
            Self::Runtime(error) => error.code(),
            Self::Transcript(error) => error.code(),
            Self::Providers(error) => error.code(),
            Self::Media(error) => error.code(),
            Self::Ai(error) => error.code(),
            Self::Io(_) => crate::session::wire::ErrorCode::Io,
        }
    }
    fn transient(&self) -> bool {
        match self {
            Self::Runtime(error) => error.transient(),
            Self::Transcript(error) => error.transient(),
            Self::Providers(error) => error.transient(),
            Self::Ai(error) => error.transient(),
            _ => false,
        }
    }
    fn job_policy(&self) -> crate::session::wire::JobFailurePolicy {
        match self {
            Self::Runtime(error) => error.job_policy(),
            Self::Transcript(error) => error.job_policy(),
            Self::Providers(error) => error.job_policy(),
            Self::Ai(error) => error.job_policy(),
            _ => crate::session::wire::JobFailurePolicy::default(),
        }
    }
}
impl From<crate::session::wire::ControlError> for Error {
    fn from(error: crate::session::wire::ControlError) -> Self {
        Self::Edit(error.into())
    }
}
impl From<crate::session::SessionError> for Error {
    fn from(error: crate::session::SessionError) -> Self {
        Self::Edit(error.into())
    }
}
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

#[derive(Clone, Debug)]
pub struct SubtitleRequest {
    pub input: PathBuf,
    /// `None` requests local whisper.cpp language detection.
    pub source_language: Option<LanguageTag>,
    pub target_language: LanguageTag,
    pub bilingual: bool,
    pub srt: Option<PathBuf>,
    pub burn: Option<PathBuf>,
    /// `Some(true)` selects 1080x1920; `Some(false)` selects 1920x1080; `None` keeps
    /// the project canvas.
    pub portrait: Option<bool>,
    pub overwrite: bool,
}

#[derive(Clone, Debug)]
pub struct SubtitleOutcome {
    pub source_language: LanguageTag,
    pub target_language: LanguageTag,
    pub transcription: TranscriptionProvider,
    pub translation: Option<TranslationProvider>,
    pub segments: usize,
    pub cues: usize,
    pub checkpoints: usize,
    pub srt: Option<PathBuf>,
    pub burn: Option<PathBuf>,
    pub transcription_cost: Cost,
    pub translation_cost: Cost,
    pub elapsed: Duration,
}

mod pipeline;

mod persistence;

pub use pipeline::run;

mod ass_import;
pub use ass_import::{AssImportReport, AssImportRequest, prepare_ass_import};

mod access;
mod generation;
mod outputs;
mod translation;
pub use access::CaptionAccess;
pub(crate) use generation::{generate, validate_source};
pub(crate) use outputs::apply_command;
pub use outputs::{decode_captions, decode_translations};
pub(crate) use translation::translate;
