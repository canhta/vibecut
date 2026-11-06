//! Measures source media without mutating projects.
mod scene_detection_workflow;
pub mod scene_detection {
    //! Deterministic source-time scene candidates; analysis never mutates a project.
    pub(crate) use super::scene_detection_workflow::RECIPE;
    pub use super::scene_detection_workflow::{
        ChangeKind, DetectionJob, DetectionRequest, DetectionResult, DetectionSettings,
        SceneCandidate, apply_reviewed,
    };
}
mod shot_analysis;
pub use shot_analysis::{
    AnalysisJob, AnalysisRequest, AnalysisResult, AnalysisSettings, FaceTrack, Scene,
    SecondAnalysis, ShotAnalysis, SpeakerRef, SpeakerState,
};
mod loudness_workflow;
pub mod loudness {
    //! Two-pass EBU R128 normalization: content-bound gains, a cached measurement
    pub use super::loudness_workflow::{
        LoudnessJob, LoudnessResult, Measured, admit, content_key, submit_loudness,
    };
}
