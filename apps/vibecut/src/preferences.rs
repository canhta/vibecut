//! The one versioned preference document and the file that persists it.
//!
//! Feature-owned section types keep their own defaults and validation; this
//! module only composes them in the document the app saves.
mod aggregate;
mod font_picker;
mod geometry;
mod store;

pub use crate::services::remote_models::{AiPreferences, MAX_RECENT_MODELS, ModelChoice};
pub use aggregate::{
    DEFAULT_PANELS, MAX_PREFERENCES_BYTES, MAX_SECTIONS, MAX_TIMELINE_LANES, PANEL_MAX, PANEL_MIN,
    PREFERENCE_KEYS, PREFERENCES_VERSION, PREVIEW_MAX, PREVIEW_MIN, Preferences, TIMELINE_MAX,
    TIMELINE_MIN, drag_panel, validate_panels,
};
pub use font_picker::{FontPickerPreferences, FontTab};
pub use geometry::WindowGeometry;
pub use store::{PreferenceIssue, PreferenceStore};
