//! Native editor frontend. Device creation and window ownership stay in main.
pub mod capture;
pub mod catalog;
mod frontend;
pub mod lifecycle;
pub mod playback;
pub mod workspace;
pub use eframe::egui;
pub use frontend::{Editor, Inspection, WidgetState};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unsupported desktop capability: {0}")]
    Unsupported(&'static str),
    #[error("operation cancelled")]
    Cancelled,
    #[error("preview unavailable: {0}")]
    PreviewUnavailable(&'static str),
    #[error("desktop state is unavailable")]
    Poisoned,
    #[error("capture timed out")]
    CaptureTimeout,
    #[error("capture unavailable")]
    CaptureUnavailable,
    #[error("invalid desktop input: {0}")]
    Invalid(&'static str),
    #[error("engine operation failed")]
    Engine(#[from] engine::EditError),
    #[error("preview worker failed")]
    Worker(#[source] std::sync::Arc<engine::EditError>),
    #[error("project operation failed")]
    Project(#[from] project::Error),
    #[error("render operation failed")]
    Render(#[from] render::Error),
    #[error("time conversion failed")]
    Time(#[from] time::TimeError),
    #[error("desktop IO failed")]
    Io(#[from] std::io::Error),
    #[error("preferences could not be read")]
    Preferences(#[from] serde_json::Error),
    #[error("native window or GPU failed")]
    Window(#[from] eframe::Error),
    #[error("audio device failed")]
    Audio(#[from] cpal::Error),
    #[error("image operation failed")]
    Image(#[from] image::ImageError),
}
pub type Result<T> = std::result::Result<T, Error>;
