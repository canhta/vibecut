//! Produces derived project artifacts.
/// Derived artifact failures retain their actual lower causes.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Edit(#[from] crate::session::EditError),
    #[error(transparent)]
    Runtime(#[from] crate::runtime::Error),
    #[error(transparent)]
    Lut(#[from] crate::luts::Error),
    #[error("render execution failed")]
    Render(#[from] render::Error),
    #[error("media operation failed")]
    Media(#[from] media::MediaError),
    #[error("AI execution failed")]
    Ai(#[from] ai::Error),
    #[error("job/cache IO failed")]
    Io(#[from] std::io::Error),
    #[error("Project is empty")]
    EmptyProject,
}
pub type Result<T> = std::result::Result<T, Error>;
impl crate::session::wire::Failure for Error {
    fn code(&self) -> crate::session::wire::ErrorCode {
        match self {
            Self::Edit(error) => error.code(),
            Self::Runtime(error) => error.code(),
            Self::Lut(error) => error.code(),
            Self::Render(error) => error.code(),
            Self::Media(error) => error.code(),
            Self::Ai(error) => error.code(),
            Self::Io(_) => crate::session::wire::ErrorCode::Io,
            Self::EmptyProject => crate::session::wire::ErrorCode::EmptyProject,
        }
    }
    fn transient(&self) -> bool {
        match self {
            Self::Runtime(error) => error.transient(),
            Self::Lut(error) => error.transient(),
            Self::Ai(error) => error.transient(),
            _ => false,
        }
    }
    fn job_policy(&self) -> crate::session::wire::JobFailurePolicy {
        match self {
            Self::Runtime(error) => error.job_policy(),
            Self::Lut(error) => error.job_policy(),
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
mod chapters;
pub(crate) mod lut;
pub mod chapter_sidecars {
    //! Publishes chapter and description sidecars.
    pub use super::chapters::{ChapterSidecars, sidecar_paths, write, write_to_directory};
}
mod frame_thumbnails;
pub mod thumbnails {
    //! Produces cached source thumbnails.
    pub use super::frame_thumbnails::{RetainedThumbnail, ThumbnailCache, thumbnail_key};
}
mod proxy;
mod proxy_sources;
pub mod proxies {
    //! Generates preview proxies and resolves preview reads to them.
    pub use super::proxy::{ProxyJob, ProxyReady, ProxyRequest, proxy_key};
    pub use super::proxy_sources::{ProxyFallback, ProxyIndex, ProxySources};
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
