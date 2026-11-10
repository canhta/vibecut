//! Measures source media without mutating projects.
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
