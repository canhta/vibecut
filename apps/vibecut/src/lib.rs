//! Composes the desktop application from its public interfaces.
pub mod actions;
pub mod app;
mod editor;
pub mod home;
pub mod i18n;
pub mod input;
mod kit;
pub mod preferences;
pub mod services;
pub mod shorts;
pub use editor::dialogs::export;
pub use editor::dialogs::models::model_manager;
pub use editor::dialogs::voiceover::VoiceoverView;
pub use editor::properties::ColorTarget;
pub use editor::properties::font_picker;
pub use editor::properties::{ElementKind, PropertiesTab, shared_fields};
pub use editor::{
    AppCommand, AssetsTab, Client, CropAspect, CropInspection, Editor, EffectPreviewStatus,
    EffectPreviews, Guide, ImportEvent, ImportProbe, ImportQueue, Inspection, KeyframeSelection,
    LibraryProblem, LibraryStore, MAX_IMPORT_FILES, MaskPointSelection, MediaSort, MediaSortKey,
    MediaSortOrder, MediaView, Music, MusicMode, MusicProblem, OverlayVisibility, PREVIEW_SCALE,
    PREVIEW_SIZE, PREVIEW_SNAP_PIXELS, PreviewAxis, PreviewGuide, PreviewZoom, SETTLE_BUDGET,
    SelectionScope, ShortcutTab, Sounds, SoundsMode, SoundsProblem, Subtitles, SubtitlesInspection,
    TextCaret, ThumbnailSource, WidgetState,
};
pub use eframe::egui;
pub use kit::Look;

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
    /// A sound can be starred only while it is in the Sounds results or the saved library.
    #[error("sound is not in the current results or the saved library")]
    SoundNotListed,
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
    Engine(#[from] engine::control::Error),
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

impl From<engine::providers::Error> for Error {
    fn from(error: engine::providers::Error) -> Self {
        Self::Engine(error.into())
    }
}

impl From<engine::transcript::Error> for Error {
    fn from(error: engine::transcript::Error) -> Self {
        Self::Engine(error.into())
    }
}

impl From<engine::ArtifactError> for Error {
    fn from(error: engine::ArtifactError) -> Self {
        Self::Engine(error.into())
    }
}
