//! Measures source media without mutating projects.
/// Source-media measurement failures retain their genuine lower causes.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Edit(#[from] crate::session::EditError),
    #[error(transparent)]
    Runtime(#[from] crate::runtime::Error),
    #[error(transparent)]
    Providers(#[from] crate::providers::Error),
    #[error("media operation failed")]
    Media(#[from] media::MediaError),
    #[error("render execution failed")]
    Render(#[from] render::Error),
    #[error("AI execution failed")]
    Ai(#[from] ai::Error),
}
pub type Result<T> = std::result::Result<T, Error>;
impl crate::session::wire::Failure for Error {
    fn code(&self) -> crate::session::wire::ErrorCode {
        match self {
            Self::Edit(error) => error.code(),
            Self::Runtime(error) => error.code(),
            Self::Providers(error) => error.code(),
            Self::Media(error) => error.code(),
            Self::Render(error) => error.code(),
            Self::Ai(error) => error.code(),
        }
    }
    fn transient(&self) -> bool {
        match self {
            Self::Runtime(error) => error.transient(),
            Self::Providers(error) => error.transient(),
            Self::Ai(error) => error.transient(),
            _ => false,
        }
    }
    fn job_policy(&self) -> crate::session::wire::JobFailurePolicy {
        match self {
            Self::Runtime(error) => error.job_policy(),
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
impl From<project::Error> for Error {
    fn from(error: project::Error) -> Self {
        Self::Edit(error.into())
    }
}
impl From<time::TimeError> for Error {
    fn from(error: time::TimeError) -> Self {
        Self::Edit(error.into())
    }
}
mod scene_detection_workflow;
pub mod scene_detection {
    //! Detects source-time scene-change candidates.
    pub(crate) use super::scene_detection_workflow::RECIPE;
    pub use super::scene_detection_workflow::{
        DetectionJob, DetectionRequest, DetectionSettings, apply_reviewed, decode,
    };
    pub use crate::session::wire::{ChangeKind, DetectionResult, SceneCandidate};
}
mod shot_analysis;
pub use shot_analysis::{
    AnalysisJob, AnalysisRequest, AnalysisResult, AnalysisSettings, FaceTrack, Scene,
    SecondAnalysis, ShotAnalysis, SpeakerRef, SpeakerState,
};
pub mod chapters;
pub mod fillers;
mod loudness_workflow;
pub mod pause_candidates;
pub mod speech_activity;
pub mod waveform;
pub mod loudness {
    //! Measures source audio loudness.
    pub use super::loudness_workflow::{decode, submit_loudness};
    pub use crate::runtime::{admit_loudness as admit, loudness_content_key as content_key};
    pub use crate::session::wire::{LoudnessResult, Measured};
}
