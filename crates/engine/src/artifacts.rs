//! Produces derived project artifacts.
mod chapters;
pub mod chapter_sidecars {
    //! Publishes chapter and description sidecars.
    pub use super::chapters::{ChapterSidecars, sidecar_paths, write};
}
mod frame_thumbnails;
pub mod thumbnails {
    //! Produces cached source thumbnails.
    pub use super::frame_thumbnails::{RetainedThumbnail, ThumbnailCache, thumbnail_key};
}
mod frame_snapshot;
pub mod thumbnail_strip;
pub mod snapshot {
    //! Publishes project frame snapshots.
    pub(crate) use super::frame_snapshot::save_with_overwrite;
    pub use super::frame_snapshot::{
        MAX_SNAPSHOT_MEMORY, MAX_SNAPSHOT_PNG_BYTES, SnapshotImage, SnapshotResult, default_name,
        image, render_time, save, save_with_budget,
    };
}
mod effect_images;
pub mod effect_thumbnails {
    //! Produces illustrative effect thumbnails.
    pub use super::effect_images::{EffectThumbnails, MAX_THUMBNAIL_EDGE, MAX_THUMBNAILS};
}
mod preview_execution;
pub(crate) mod control_preview {
    //! Produces bounded labelled control previews.
    pub(crate) use super::preview_execution::{control_key, preview};
}
