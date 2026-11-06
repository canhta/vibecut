//! Produces derived project images.
mod frame_thumbnails;
pub mod thumbnails {
    //! Content-keyed thumbnail artifacts.
    pub use super::frame_thumbnails::{RetainedThumbnail, ThumbnailCache, thumbnail_key};
}
mod frame_snapshot;
pub mod snapshot {
    //! Current-frame PNG snapshots over the production compositor.
    pub(crate) use super::frame_snapshot::save_with_overwrite;
    pub use super::frame_snapshot::{
        MAX_SNAPSHOT_MEMORY, MAX_SNAPSHOT_PNG_BYTES, SnapshotImage, SnapshotResult, default_name,
        image, render_time, save, save_with_budget,
    };
}
mod effect_images;
pub mod effect_thumbnails {
    //! Device-injected offscreen effect thumbnails, scheduled by the engine.
    pub use super::effect_images::{EffectThumbnails, MAX_THUMBNAIL_EDGE, MAX_THUMBNAILS};
}
mod preview_execution;
pub(crate) mod control_preview {
    //! Bounded, labelled previews use the production compositor and engine scheduler.
    pub use super::preview_execution::PreviewResult;
    pub(crate) use super::preview_execution::preview;
}
