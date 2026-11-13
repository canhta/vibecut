//! Produces accepted shorts from source segments.
mod error;
pub use error::{Error, Result};
mod boundaries;
pub mod clip_boundaries {
    //! Snaps accepted source segments to safe edges.
    pub use super::boundaries::{
        BoundaryAdjustment, BoundaryKind, ClipBoundarySettings, SegmentAdjustment, SnappedShort,
        snap_clip_boundaries,
    };
    pub use super::error::BoundaryFailure;
}
mod layout;
pub use layout::{Layout, LayoutEdit, LayoutOverrides, ShotLayout};
mod batch_export;
pub use batch_export::{
    ClipBatch, ClipBatchRequest, ClipPaths, ClipShort, ClipSubtitlePath, MAX_BATCH_CLIPS,
    MAX_BATCH_CUE_BYTES, MAX_CLIP_SEGMENTS,
};
mod suggestion;
pub mod clip_suggest {
    //! Produces source-snapped clip suggestions.
    pub use super::suggestion::{
        ClipCandidate, OPENAI_POLICY_URL, RejectedClip, SnapSettings, SuggestionOption,
        SuggestionProvider, candidate_view, check_suggestion_credentials, choose_suggestion,
        segments_from_view, suggestion_options,
    };
}

pub use clip_suggest::{
    ClipCandidate, RejectedClip, SnapSettings, SuggestionOption, SuggestionProvider,
    check_suggestion_credentials, choose_suggestion, suggestion_options,
};
mod vision;
pub use vision::{ModelLookup, installed_faces};
mod workflow;
pub use workflow::ClipJobs;
