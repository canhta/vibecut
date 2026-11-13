//! The one versioned preference document and the file that persists it.
//!
//! Feature-owned section types keep their own defaults and validation; this
//! module only composes them in the document the app saves.
mod adjustment;
mod aggregate;
mod easing;
mod font_picker;
mod geometry;
mod store;
mod subtitles;

pub use crate::services::remote_models::{AiPreferences, MAX_RECENT_MODELS, ModelChoice};
pub use adjustment::{
    AdjustmentPreset, MAX_ADJUSTMENT_PRESET_NAME, MAX_ADJUSTMENT_PRESETS,
    validate_adjustment_presets,
};
pub use aggregate::{
    DEFAULT_PANELS, MAX_PREFERENCES_BYTES, MAX_SECTIONS, MAX_TIMELINE_LANES, PANEL_MAX, PANEL_MIN,
    PREFERENCE_KEYS, PREFERENCES_VERSION, PREVIEW_MAX, PREVIEW_MIN, Preferences, TIMELINE_MAX,
    TIMELINE_MIN, drag_panel, validate_panels,
};
pub use easing::{EasingPreferences, EasingTab, MAX_SAVED_EASINGS, SavedEasing};
pub use font_picker::{FontPickerPreferences, FontTab};
pub use geometry::WindowGeometry;
pub use store::{PreferenceIssue, PreferenceStore};
pub use subtitles::{Options as SubtitleOptions, SOURCE_LANGUAGES};
