//! Produces accepted shorts from source segments.
mod boundaries;
pub mod clip_boundaries {
    //! Independent source-edge snapping for ordered, potentially reordered shorts.
    pub use super::boundaries::{
        BoundaryAdjustment, BoundaryFailure, BoundaryKind, ClipBoundarySettings, SegmentAdjustment,
        SnappedShort, snap_clip_boundaries,
    };
}
mod batch_export;
pub use batch_export::{
    ClipBatch, ClipBatchRequest, ClipPaths, ClipShort, ClipSubtitlePath, MAX_BATCH_CLIPS,
    MAX_BATCH_CUE_BYTES, MAX_CLIP_SEGMENTS,
};
mod suggestion;
pub mod clip_suggest {
    //! Clip suggestion workflow: transcript segments in, ranked shorts with snapped
    pub use super::suggestion::{
        ClipCandidate, ClipExportOutcome, ClipExportPaths, ClipExportPrep, ClipExportRequest,
        ClipRequest, ClipSuggestionOutcome, OPENAI_POLICY_URL, RejectedClip, SnapSettings,
        SuggestionOption, SuggestionProvider, check_suggestion_credentials, choose_suggestion,
        export, prepare_export, suggest, suggestion_options,
    };
}
