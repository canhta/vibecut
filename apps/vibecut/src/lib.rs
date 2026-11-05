//! Native editor frontend. Device creation and window ownership stay in main.
mod ai_task;
pub mod capture;
pub mod catalog;
pub mod crop;
pub mod effect_previews;
pub mod effects;
pub mod export;
mod frontend;
mod icons;
pub mod import;
pub mod lifecycle;
pub mod media_panel;
pub mod model_manager;
pub mod panels;
pub mod playback;
pub mod projects;
pub mod preview;
pub mod selection;
pub mod shorts;
pub mod snapping;
pub mod subtitles;
mod theme;
pub mod workspace;
pub use eframe::egui;
pub use frontend::{
    AssetInspection, AssetState, Editor, Inspection, PREFERENCE_COMMAND, WidgetState,
};

/// Window background token. Host adapters and offscreen audits paint it behind a
/// shell that only draws its own content (Home does not fill the window).
pub fn window_color(dark: bool) -> egui::Color32 {
    theme::window(dark)
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("empty timeline cannot be exported")]
    EmptyTimeline,
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
    #[error("AI operation failed")]
    Ai(#[from] ai::Error),
    #[error("caption data failed")]
    Captions(#[from] captions::Error),
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
