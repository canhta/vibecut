//! Produces accepted shorts from source segments.
mod boundaries;
pub mod clip_boundaries {
    //! Snaps accepted source segments to safe edges.
    pub use super::boundaries::{
        BoundaryAdjustment, BoundaryKind, ClipBoundarySettings, SegmentAdjustment, SnappedShort,
        snap_clip_boundaries,
    };
    pub use crate::BoundaryFailure;
}
mod batch_export;
pub use batch_export::{
    ClipBatch, ClipBatchRequest, ClipPaths, ClipShort, ClipSubtitlePath, MAX_BATCH_CLIPS,
    MAX_BATCH_CUE_BYTES, MAX_CLIP_SEGMENTS,
};
mod suggestion;
pub mod clip_suggest {
    //! Produces source-snapped clip suggestions.
    pub use super::suggestion::{
        ClipCandidate, ClipExportOutcome, ClipExportPaths, ClipExportPrep, ClipExportRequest,
        ClipRequest, ClipSuggestionOutcome, OPENAI_POLICY_URL, RejectedClip, SnapSettings,
        SuggestionOption, SuggestionProvider, check_suggestion_credentials, choose_suggestion,
        export, prepare_export, suggest, suggestion_options,
    };
}
