//! The versioned preference document composed from its feature-owned sections.

use super::performance::{
    DEFAULT_CACHE_LIMIT, DEFAULT_PROJECT_FPS, HardwareAcceleration, validate_cache_limit,
    validate_project_fps,
};
use super::{
    AdjustmentPreset, CatalogFavorites, EasingPreferences, FontPickerPreferences, WindowGeometry,
};
use crate::{
    ElementKind, PropertiesTab,
    editor::AssetsTab,
    editor::{Guide, GuideLines, MediaSort, MediaView, OverlayVisibility},
    home::ProjectsPreferences,
    i18n::Locale,
    input::KeybindingPreferences,
};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// Bumped when the local preference document changes incompatibly.
pub const PREFERENCES_VERSION: u32 = 21;
/// Room for every provider/feature choice and its bounded recent IDs.
pub const MAX_PREFERENCES_BYTES: u64 = 32 * 1024;
/// Total open-lane entries across every project; keeps the document bounded.
pub const MAX_TIMELINE_LANES: usize = 4096;

pub const MAX_SECTIONS: usize = 128;

/// Column minimums in logical px, measured from CapCut 9.7.1 (capture
/// 2026-11-14c): identical at 1840 x 1142 and at the 1268 x 650 minimum window.
/// A column's maximum is what the other two leave at their minimums.
pub const MEDIA_MIN_PX: f32 = 398.0;
pub const PLAYER_MIN_PX: f32 = 398.0;
pub const PROPERTIES_MIN_PX: f32 = 368.0;
// Timeline limits from CapCut 9.7.1 frames 111948 (one track row, 408 native px of a
// 2182 px row height, 204 pt) and 111942 (full height, 1670 px, 835 pt), sampled at
// native resolution (logical = px / 2). Row height is 1091 pt at 1840 x 1142.
pub const TIMELINE_MIN: f32 = 0.187;
pub const TIMELINE_MAX: f32 = 0.765;
// Widths exclude both gutters; row fractions include the area-owned strips.
/// First open at 1840 x 1142 (capture 2026-11-14c): media 398, player 1031, properties
/// 379 pt of 1809 pt columns (layout.rs: 1823 pt body minus two 7 pt gutters). Side
/// columns are stored as fractions of the columns so a resize scales them
/// proportionally; `column_widths` applies the pixel minimums. Timeline 498 pt of
/// 1091 pt (frame 112001, x=300 scan), i.e. 447 / 979.
pub const DEFAULT_PANELS: [f32; 3] = [398.0 / 1809.0, 379.0 / 1809.0, 447.0 / 979.0];

/// Every user-visible preference key in the one versioned document.
pub const PREFERENCE_KEYS: &[&str] = &[
    "ai_models",
    "locale",
    "dark",
    "snapping",
    "ripple",
    "panels",
    "sections",
    "export",
    "assets_tab",
    "properties_tabs",
    "projects",
    "media_view",
    "media_sort",
    "preview_fit",
    "preview_zoom",
    "preview_axis",
    "playback_loop",
    "keybindings",
    "timeline_lanes",
    "keyframe_hint_seen",
    "font_picker",
    "adjustment_presets",
    "sticker_recents",
    "catalog_favorites",
    "easing",
    "default_fps",
    "hardware_acceleration",
    "cache_limit_bytes",
    "preview_guide",
    "preview_overlays",
    "preview_lines",
    "track_height",
    "waveform_size",
];

/// Media, player and properties widths in px for `columns` px of room: the stored
/// fractions scaled to the window, each side column held at its pixel minimum and
/// at most what the other two leave at theirs. The player takes the rest.
pub fn column_widths(panels: [f32; 3], columns: f32) -> [f32; 3] {
    let columns = columns.max(0.0);
    let floor = MEDIA_MIN_PX + PLAYER_MIN_PX + PROPERTIES_MIN_PX;
    if columns < floor {
        let scale = columns / floor;
        let media = MEDIA_MIN_PX * scale;
        let properties = PROPERTIES_MIN_PX * scale;
        return [media, columns - media - properties, properties];
    }
    let media =
        (panels[0] * columns).clamp(MEDIA_MIN_PX, columns - PLAYER_MIN_PX - PROPERTIES_MIN_PX);
    let properties =
        (panels[1] * columns).clamp(PROPERTIES_MIN_PX, columns - PLAYER_MIN_PX - media);
    [media, columns - media - properties, properties]
}

/// A splitter drag, `delta` as a fraction of the columns (axes 0, 1) or of the
/// lower row (axis 2), that keeps every region inside its limits. `columns` is the
/// px room the side columns and player share.
pub fn drag_panel(panels: [f32; 3], axis: usize, delta: f32, columns: f32) -> [f32; 3] {
    let Ok(mut next) = validate_panels(panels) else {
        return DEFAULT_PANELS;
    };
    if !delta.is_finite() {
        return next;
    }
    let floor = MEDIA_MIN_PX + PLAYER_MIN_PX + PROPERTIES_MIN_PX;
    if axis < 2 && !(columns.is_finite() && columns >= floor) {
        return next;
    }
    match axis {
        0 => {
            let [media, _, properties] = column_widths(next, columns);
            let lo = MEDIA_MIN_PX / columns;
            let hi = ((columns - PLAYER_MIN_PX - properties) / columns).max(lo);
            next[0] = (media / columns + delta).clamp(lo, hi);
            next[1] = properties / columns;
        }
        1 => {
            let [media, _, properties] = column_widths(next, columns);
            let lo = PROPERTIES_MIN_PX / columns;
            let hi = ((columns - PLAYER_MIN_PX - media) / columns).max(lo);
            next[1] = (properties / columns - delta).clamp(lo, hi);
            next[0] = media / columns;
        }
        2 => next[2] = (next[2] - delta).clamp(TIMELINE_MIN, TIMELINE_MAX),
        _ => return next,
    }
    next
}

/// Fractions are finite and leave room for the player; the pixel minimums apply at
/// layout time (`column_widths`), where the window width is known.
pub fn validate_panels(panels: [f32; 3]) -> Result<[f32; 3]> {
    if panels.iter().any(|value| !value.is_finite()) {
        return Err(Error::Invalid("panel ratios"));
    }
    let [assets, properties, timeline] = panels;
    if assets <= 0.0
        || properties <= 0.0
        || assets + properties >= 1.0
        || !(TIMELINE_MIN..=TIMELINE_MAX).contains(&timeline)
    {
        return Err(Error::Invalid("panel ratios"));
    }
    Ok(panels)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub ai_models: super::AiPreferences,
    pub version: u32,
    pub windows: BTreeMap<String, WindowGeometry>,
    pub export: crate::export::ExportOptions,
    pub export_directory: Option<PathBuf>,
    pub subtitles: super::SubtitleOptions,
    pub locale: Locale,
    pub dark: bool,
    /// Timeline snapping; on by default and persisted across runs.
    pub snapping: bool,
    /// Close vacated same-track tails after timeline edits; off by default.
    pub ripple: bool,
    /// Ratios of the full content rectangle; validated when loaded and set.
    pub panels: [f32; 3],
    /// Open/closed state by property-section key.
    pub sections: BTreeMap<String, bool>,
    /// Last assets-panel tab. Presentation state, never a project edit.
    pub assets_tab: AssetsTab,
    /// Last properties tab by persisted element-kind id. Applying it to a kind
    /// that does not offer that tab falls back to the kind's default.
    pub properties_tabs: BTreeMap<String, String>,
    /// Projects page view mode and sort order; presentation only.
    pub projects: ProjectsPreferences,
    /// Media library grid/list presentation.
    pub media_view: MediaView,
    /// Media library sort key and direction.
    pub media_sort: MediaSort,
    /// Preview follows the panel size when true; otherwise `preview_zoom` applies.
    pub preview_fit: bool,
    /// Explicit preview scale where 1.0 is 100%; clamped to 0.25–16.
    pub preview_zoom: f64,
    /// Hovering the timeline scrubs the preview to the pointer time; on by default.
    pub preview_axis: bool,
    /// Loop playback over the export in/out range, or the whole timeline when unset.
    pub playback_loop: bool,
    /// Last directory used by the snapshot save dialog.
    pub snapshot_directory: Option<PathBuf>,
    /// App-local action overrides. Defaults and action metadata live in
    /// `actions`; this is the only persisted binding map.
    pub keybindings: KeybindingPreferences,
    /// Items whose keyframe lanes stay open, keyed by the project they belong
    /// to: item ids restart in every project.
    pub timeline_lanes: BTreeMap<project::ProjectId, BTreeSet<u64>>,
    /// The first-use keyframe hint was dismissed; presentation memory, never project data.
    pub keyframe_hint_seen: bool,
    /// Font picker tab and favourite families; presentation only.
    pub font_picker: FontPickerPreferences,
    /// Named colour-adjustment presets stored with the app.
    pub adjustment_presets: Vec<AdjustmentPreset>,
    /// Recently inserted sticker ids, newest first and deduplicated; presentation memory, never project data.
    pub sticker_recents: Vec<String>,
    /// Starred catalogue ids per family (effects, transitions, ...); presentation memory, never project data.
    pub catalog_favorites: CatalogFavorites,
    /// Graph editor tab and saved easing curves; user data, never project state.
    pub easing: EasingPreferences,
    /// Frame rate new projects start with; one of `PROJECT_FPS`.
    pub default_fps: u32,
    /// Hardware decode/encode choice; applies after the next start.
    pub hardware_acceleration: HardwareAcceleration,
    /// Aggregate byte budget for frames, compositor, export and cache; applies after the next start.
    pub cache_limit_bytes: u64,
    /// The preview guide the picker selected; drawn by the app, never exported.
    pub preview_guide: Guide,
    /// Visibility of each preview overlay; presentation only.
    pub preview_overlays: OverlayVisibility,
    /// The player's rulers and custom guide lines; drawn by the app, never exported.
    pub preview_lines: GuideLines,
    /// Timeline row height choice (track header menu); presentation only.
    pub track_height: super::TrackHeight,
    /// Audio waveform size in percent, one of `WAVEFORM_SIZES`; presentation only.
    pub waveform_size: u8,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            ai_models: super::AiPreferences::default(),
            version: PREFERENCES_VERSION,
            windows: BTreeMap::new(),
            export: crate::export::ExportOptions::default(),
            export_directory: None,
            subtitles: super::SubtitleOptions::default(),
            locale: Locale::En,
            dark: true,
            snapping: true,
            ripple: false,
            panels: DEFAULT_PANELS,
            sections: BTreeMap::new(),
            assets_tab: AssetsTab::default(),
            properties_tabs: BTreeMap::new(),
            projects: ProjectsPreferences::default(),
            media_view: MediaView::default(),
            media_sort: MediaSort::default(),
            preview_fit: true,
            preview_zoom: 1.0,
            preview_axis: true,
            playback_loop: false,
            snapshot_directory: None,
            keybindings: KeybindingPreferences::default(),
            timeline_lanes: BTreeMap::new(),
            keyframe_hint_seen: false,
            font_picker: FontPickerPreferences::default(),
            adjustment_presets: Vec::new(),
            sticker_recents: Vec::new(),
            catalog_favorites: CatalogFavorites::default(),
            easing: EasingPreferences::default(),
            default_fps: DEFAULT_PROJECT_FPS,
            hardware_acceleration: HardwareAcceleration::default(),
            cache_limit_bytes: DEFAULT_CACHE_LIMIT,
            preview_guide: Guide::default(),
            preview_overlays: OverlayVisibility::default(),
            preview_lines: GuideLines::default(),
            track_height: super::TrackHeight::default(),
            waveform_size: super::DEFAULT_WAVEFORM_SIZE,
        }
    }
}
impl Preferences {
    /// The copied appearance value windows draw with.
    pub fn look(&self) -> crate::kit::Look {
        crate::kit::Look {
            locale: self.locale,
            dark: self.dark,
        }
    }
    /// First launch follows the operating system language.
    pub fn system() -> Self {
        Self {
            locale: Locale::from_system(),
            ..Self::default()
        }
    }
    pub fn validate(&self) -> Result<()> {
        self.ai_models.validate()?;
        if self.version != PREFERENCES_VERSION {
            return Err(Error::Unsupported("preferences version"));
        }
        if self.windows.len() > 64 || self.windows.iter().any(|(k, v)| k.len() > 64 || !v.valid()) {
            return Err(Error::Invalid("window geometry"));
        }
        validate_panels(self.panels)?;
        if !super::WAVEFORM_SIZES.contains(&self.waveform_size) {
            return Err(Error::Invalid("waveform size"));
        }
        self.subtitles.validate()?;
        self.export.output.validate()?;
        self.validate_sections()?;
        self.validate_properties_tabs()?;
        if !self.preview_zoom.is_finite()
            || !(f64::from(crate::services::preview::MIN_ZOOM)
                ..=f64::from(crate::services::preview::MAX_ZOOM))
                .contains(&self.preview_zoom)
        {
            return Err(Error::Invalid("preview zoom"));
        }
        if self
            .snapshot_directory
            .as_ref()
            .is_some_and(|path| path.as_os_str().len() > crate::services::preview::MAX_PATH_BYTES)
        {
            return Err(Error::Invalid("snapshot directory"));
        }
        self.keybindings.validate()?;
        if self.timeline_lane_count() > MAX_TIMELINE_LANES {
            return Err(Error::Invalid("timeline lanes"));
        }
        self.font_picker.validate()?;
        super::adjustment::validate_adjustment_presets(&self.adjustment_presets)?;
        if self.sticker_recents.len() > libraries::stickers::MAX_RECENT_STICKERS {
            return Err(Error::Invalid("sticker recents"));
        }
        self.catalog_favorites.validate()?;
        self.easing.validate()?;
        validate_project_fps(self.default_fps)?;
        validate_cache_limit(self.cache_limit_bytes)?;
        self.preview_guide.validate()?;
        self.preview_lines.validate()?;
        if serde_json::to_vec(self)?.len() as u64 > MAX_PREFERENCES_BYTES {
            return Err(Error::Invalid("preference size"));
        }
        Ok(())
    }
    pub(crate) fn validate_properties_tabs(&self) -> Result<()> {
        if self.properties_tabs.len() > ElementKind::ALL.len() {
            return Err(Error::Invalid("preference tabs"));
        }
        for (kind, tab) in &self.properties_tabs {
            let kind = ElementKind::from_id(kind).ok_or(Error::Invalid("preference tabs"))?;
            let tab = PropertiesTab::from_id(tab).ok_or(Error::Invalid("preference tabs"))?;
            if !kind.tabs().contains(&tab) {
                return Err(Error::Invalid("preference tabs"));
            }
        }
        Ok(())
    }
    pub(crate) fn validate_sections(&self) -> Result<()> {
        if self.sections.len() > MAX_SECTIONS
            || self
                .sections
                .keys()
                .any(|key| key.is_empty() || key.len() > 64)
        {
            return Err(Error::Invalid("preference sections"));
        }
        Ok(())
    }
    pub fn section_open(&self, key: &str, default: bool) -> bool {
        self.sections.get(key).copied().unwrap_or(default)
    }
    /// Forget lane memory for projects `keep` rejects; the caller decides
    /// existence (see `Workspace::prune_lane_preferences`).
    pub fn retain_timeline_lanes(&mut self, keep: impl Fn(project::ProjectId) -> bool) {
        self.timeline_lanes.retain(|project, _| keep(*project));
    }
    /// Save As mints a fresh id while the issuing window edits the copy; carry
    /// that project's open-lane set to the copy and leave the original's entry
    /// untouched. A fresh id normally has no entry, so nothing is merged. The
    /// document bound wins: only entries that fit are carried.
    pub fn copy_timeline_lanes(&mut self, from: project::ProjectId, to: project::ProjectId) {
        if from == to {
            return;
        }
        let Some(source) = self.timeline_lanes.get(&from).cloned() else {
            return;
        };
        let mut total = self.timeline_lane_count();
        if let Some(previous) = self.timeline_lanes.get(&to) {
            total = total.saturating_sub(previous.len());
        }
        let room = MAX_TIMELINE_LANES.saturating_sub(total);
        let carried: BTreeSet<u64> = source.into_iter().take(room).collect();
        if carried.is_empty() {
            self.timeline_lanes.remove(&to);
        } else {
            self.timeline_lanes.insert(to, carried);
        }
    }
    pub(crate) fn timeline_lane_count(&self) -> usize {
        self.timeline_lanes
            .values()
            .map(BTreeSet::len)
            .try_fold(0usize, usize::checked_add)
            .unwrap_or(usize::MAX)
    }
    /// Typed `ui.prefs.set` payload access used by the app history.
    pub fn get(&self, key: &str) -> Result<serde_json::Value> {
        match key {
            "ai_models" => Ok(serde_json::to_value(&self.ai_models)?),
            "locale" => Ok(serde_json::to_value(self.locale)?),
            "dark" => Ok(serde_json::Value::Bool(self.dark)),
            "snapping" => Ok(serde_json::Value::Bool(self.snapping)),
            "ripple" => Ok(serde_json::Value::Bool(self.ripple)),
            "panels" => Ok(serde_json::to_value(self.panels)?),
            "sections" => Ok(serde_json::to_value(&self.sections)?),
            "export" => Ok(serde_json::to_value(self.export)?),
            "assets_tab" => Ok(serde_json::to_value(self.assets_tab)?),
            "properties_tabs" => Ok(serde_json::to_value(&self.properties_tabs)?),
            "subtitles" => Ok(serde_json::to_value(&self.subtitles)?),
            "projects" => Ok(serde_json::to_value(self.projects)?),
            "media_view" => Ok(serde_json::to_value(self.media_view)?),
            "media_sort" => Ok(serde_json::to_value(self.media_sort)?),
            "preview_fit" => Ok(serde_json::Value::Bool(self.preview_fit)),
            "preview_zoom" => Ok(serde_json::json!(self.preview_zoom)),
            "preview_axis" => Ok(serde_json::Value::Bool(self.preview_axis)),
            "playback_loop" => Ok(serde_json::Value::Bool(self.playback_loop)),
            "snapshot_directory" => Ok(serde_json::to_value(&self.snapshot_directory)?),
            "keybindings" => Ok(serde_json::to_value(&self.keybindings)?),
            "timeline_lanes" => Ok(serde_json::to_value(&self.timeline_lanes)?),
            "keyframe_hint_seen" => Ok(serde_json::Value::Bool(self.keyframe_hint_seen)),
            "font_picker" => Ok(serde_json::to_value(&self.font_picker)?),
            "adjustment_presets" => Ok(serde_json::to_value(&self.adjustment_presets)?),
            "sticker_recents" => Ok(serde_json::to_value(&self.sticker_recents)?),
            "catalog_favorites" => Ok(serde_json::to_value(&self.catalog_favorites)?),
            "easing" => Ok(serde_json::to_value(&self.easing)?),
            "default_fps" => Ok(serde_json::json!(self.default_fps)),
            "hardware_acceleration" => Ok(serde_json::to_value(self.hardware_acceleration)?),
            "cache_limit_bytes" => Ok(serde_json::json!(self.cache_limit_bytes)),
            "preview_guide" => Ok(serde_json::to_value(self.preview_guide)?),
            "preview_overlays" => Ok(serde_json::to_value(self.preview_overlays)?),
            "preview_lines" => Ok(serde_json::to_value(&self.preview_lines)?),
            "track_height" => Ok(serde_json::to_value(self.track_height)?),
            "waveform_size" => Ok(serde_json::json!(self.waveform_size)),
            _ => Err(Error::Unsupported("preference key")),
        }
    }
    pub fn set(&mut self, key: &str, value: serde_json::Value) -> Result<()> {
        let mut next = self.clone();
        next.set_value(key, value)?;
        next.resolve_models();
        next.validate()?;
        *self = next;
        Ok(())
    }
    fn set_value(&mut self, key: &str, value: serde_json::Value) -> Result<()> {
        match key {
            "ai_models" => self.ai_models = serde_json::from_value(value)?,
            "locale" => self.locale = serde_json::from_value(value)?,
            "dark" => self.dark = serde_json::from_value(value)?,
            "snapping" => self.snapping = serde_json::from_value(value)?,
            "ripple" => self.ripple = serde_json::from_value(value)?,
            "panels" => self.panels = validate_panels(serde_json::from_value(value)?)?,
            "sections" => {
                self.sections = serde_json::from_value(value)?;
            }
            "export" => self.export = serde_json::from_value(value)?,
            "assets_tab" => self.assets_tab = serde_json::from_value(value)?,
            "properties_tabs" => self.properties_tabs = serde_json::from_value(value)?,
            "subtitles" => self.subtitles = serde_json::from_value(value)?,
            "projects" => self.projects = serde_json::from_value(value)?,
            "media_view" => self.media_view = serde_json::from_value(value)?,
            "media_sort" => self.media_sort = serde_json::from_value(value)?,
            "preview_fit" => self.preview_fit = serde_json::from_value(value)?,
            "preview_zoom" => self.preview_zoom = serde_json::from_value(value)?,
            "preview_axis" => self.preview_axis = serde_json::from_value(value)?,
            "playback_loop" => self.playback_loop = serde_json::from_value(value)?,
            "snapshot_directory" => self.snapshot_directory = serde_json::from_value(value)?,
            "keybindings" => self.keybindings = serde_json::from_value(value)?,
            "timeline_lanes" => self.timeline_lanes = serde_json::from_value(value)?,
            "keyframe_hint_seen" => self.keyframe_hint_seen = serde_json::from_value(value)?,
            "font_picker" => self.font_picker = serde_json::from_value(value)?,
            "adjustment_presets" => self.adjustment_presets = serde_json::from_value(value)?,
            "sticker_recents" => self.sticker_recents = serde_json::from_value(value)?,
            "catalog_favorites" => self.catalog_favorites = serde_json::from_value(value)?,
            "easing" => self.easing = serde_json::from_value(value)?,
            "default_fps" => self.default_fps = serde_json::from_value(value)?,
            "hardware_acceleration" => {
                self.hardware_acceleration = serde_json::from_value(value)?;
            }
            "cache_limit_bytes" => self.cache_limit_bytes = serde_json::from_value(value)?,
            "preview_guide" => self.preview_guide = serde_json::from_value(value)?,
            "preview_overlays" => self.preview_overlays = serde_json::from_value(value)?,
            "preview_lines" => self.preview_lines = serde_json::from_value(value)?,
            "track_height" => self.track_height = serde_json::from_value(value)?,
            "waveform_size" => self.waveform_size = serde_json::from_value(value)?,
            _ => return Err(Error::Unsupported("preference key")),
        }
        Ok(())
    }
    /// Refresh transient workflow snapshots from the one persisted model setting.
    pub fn resolve_models(&mut self) {
        if matches!(
            self.subtitles.transcription.as_str(),
            "whisper.cpp" | "whisper" | "gipformer"
        ) {
            self.subtitles.transcription_model = self.subtitles.local_transcription_model.clone();
        } else {
            self.subtitles.transcription_model = self
                .ai_models
                .model(&self.subtitles.transcription, ai::Operation::Transcribe)
                .into();
        }
        self.subtitles.translation_model = self
            .ai_models
            .model(&self.subtitles.text_provider, ai::Operation::Translate)
            .into();
        self.subtitles.fillers_model = self
            .ai_models
            .model(
                &self.subtitles.text_provider,
                ai::Operation::ClassifyFillers,
            )
            .into();
        self.subtitles.emphasis_model = self
            .ai_models
            .model(&self.subtitles.text_provider, ai::Operation::SelectEmphasis)
            .into();
        self.subtitles.chapters_model = self
            .ai_models
            .model(
                &self.subtitles.text_provider,
                ai::Operation::GenerateChapters,
            )
            .into();
    }
}
