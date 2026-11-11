//! Composes the desktop application from its public interfaces.
pub mod actions;
pub mod appearance;
pub mod capture;
pub mod control;
mod editor;
pub mod effect_previews;
pub mod effects;
pub mod home;
pub mod i18n;
pub mod import;
pub mod input;
mod kit;
pub mod lifecycle;
pub mod media_panel;
pub mod panels;
pub mod preferences;
pub mod selection;
pub mod services;
pub mod shorts;
pub mod snapping;
pub mod subtitles;
pub mod workspace;
pub use editor::dialogs::export;
pub use editor::dialogs::models::model_manager;
pub use editor::properties::ColorTarget;
pub use editor::properties::font_picker;
pub use editor::properties::{ElementKind, PropertiesTab, SharedField, shared_fields};
pub use editor::{
    AppCommand, AssetInspection, AssetState, Client, CropAspect, CropInspection, Editor,
    Inspection, PreviewAxis, PreviewGuide, PreviewZoom, ShortcutRow, ShortcutTab,
    ShortcutsInspection, Sounds, SoundsMode, SoundsProblem, SoundsView, TextCaret, WidgetState,
};
pub use eframe::egui;

/// Window background token. Host adapters and offscreen audits paint it behind a
/// shell that only draws its own content (Home does not fill the window).
pub fn window_color(dark: bool) -> egui::Color32 {
    kit::window(dark)
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("empty timeline cannot be exported")]
    EmptyTimeline,
    #[error("unsupported desktop capability: {0}")]
    Unsupported(&'static str),
    #[error("action is disabled: {0}")]
    ActionDisabled(&'static str),
    #[error("not available yet: {0}")]
    NotAvailable(&'static str),
    #[error("keybinding failed")]
    Keybinding(#[from] crate::actions::KeybindingError),
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
    Engine(#[from] engine::Error),
    #[error(transparent)]
    Shorts(#[from] ::shorts::Error),
    #[error("AI operation failed")]
    Ai(#[from] ai::Error),
    #[error("caption data failed")]
    Captions(#[from] captions::Error),
    #[error("preview worker failed")]
    Worker(#[source] std::sync::Arc<dyn engine::Failure + 'static>),
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

impl From<engine::recovery::Error> for Error {
    fn from(error: engine::recovery::Error) -> Self {
        Self::Engine(error.into())
    }
}

impl From<engine::RuntimeError> for Error {
    fn from(error: engine::RuntimeError) -> Self {
        Self::Engine(error.into())
    }
}

impl From<engine::EditError> for Error {
    fn from(error: engine::EditError) -> Self {
        Self::Engine(error.into())
    }
}

impl From<engine::models::Error> for Error {
    fn from(error: engine::models::Error) -> Self {
        Self::Engine(error.into())
    }
}
