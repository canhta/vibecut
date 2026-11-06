//! Coordinates the editor through area interfaces.
mod assets;
mod captions;
mod export;
mod frame;
pub(crate) use frame::Context;
mod models;
mod player;
mod properties;
mod timeline;
mod widgets;

mod state;
use state::{
    AUDIO_GAIN_SCRUB, AssetRow, BookmarkDrag, DragMode, DragOrigin, DragState, EffectReorder,
    MediaMarquee, MediaReveal, OPACITY_SCRUB, PreferenceCommand, PreferenceHistory,
    SHORTCUT_CONTROL_HEIGHT, SHORTCUT_ROW_HEIGHT, SHORTCUT_TEXT_SIZE, SHORTCUT_TITLE_SIZE,
    SPEED_SCRUB, ScrubDrag, ScrubSpec, ScrubTarget, TEXT_SIZE_SCRUB, Toast,
};
pub use state::{
    AssetInspection, AssetState, Editor, Inspection, ShortcutRow, ShortcutTab, ShortcutsInspection,
    WidgetState,
};

mod timing;
use timing::{display_short_time, display_time, drop_frame};

use crate::color_picker::{ColorTarget, EyedropperState, Picker};
use crate::selection::{SelectionScope, SelectionScopes, shared_fields};
use crate::snapping::{SnapCandidate, SnapSource, snap_point};
use crate::{
    Error, Result,
    actions::{
        self, ActionTarget, BlockedReason, Dispatch, DispatchContext, EngineAction,
        KeybindingIssue, Keymap, UiAction,
    },
    catalog::{self, Locale, Preferences, Text, drag_panel},
    egui::{self, Align2, Color32, FontId, Id, Rect, Vec2},
    import::{
        ImportEvent, ImportFailure, ImportProbe, ImportQueue, ThumbnailOutcome, ThumbnailSource,
    },
    kit as theme,
    kit::{self as icons, Icon},
    media_panel::{self, MediaKind, MediaSort, MediaSortKey, MediaSortOrder, MediaView},
    panels::{AssetsTab, ElementKind, PropertiesTab},
    playback::Playback,
    preview,
};
use engine::{
    Command, CreatedId, ItemParamPatch, NewItem, Session,
    jobs::{Clock, SystemClock},
};
use media::{AssetId, ContentHash, SourceAsset, StreamKind, UnsupportedReason};
use project::{BookmarkId, ItemId, ItemParams, TrackId, TrackKind};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use time::{FrameRate, Rounding, SourceRange, TICKS_PER_SECOND, Tick};

const TOOLBAR_HEIGHT: f32 = theme::size::TOOLBAR;
const BUTTON_ICON: f32 = theme::icon::DENSE;
const ICON_BUTTON: f32 = theme::size::CONTROL;
const RULER_HEIGHT: f32 = theme::size::RULER;
const LABEL_WIDTH: f32 = theme::size::TRACK_LABEL;
const SCROLLBAR: f32 = theme::size::SCROLLBAR;
const TRACK_GAP: f32 = theme::space::SM;
const MEDIA_CARD_WIDTH: f32 = theme::size::MEDIA_THUMB_WIDTH;
// The legacy card stacks thumbnail, name and kind rows at fixed offsets; the
// assets ticket replaces it with the measured thumbnail-plus-label card.
const MEDIA_CARD_HEIGHT: f32 = 120.0;
const MEDIA_CARD_GAP: f32 = theme::space::MD;
const MEDIA_ROW_HEIGHT: f32 = theme::size::AUDIO_ROW;
const MEDIA_REVEAL: Duration = Duration::from_millis(1500);
/// Fixed reference scale for persisted view state: 1× is 40 logical px/s.
const DEFAULT_ZOOM: f32 = 40.0;
/// The timeline slider tops out at 100× the reference scale.
const MAX_ZOOM: f32 = DEFAULT_ZOOM * 100.0;
/// A positive floor for hostile/empty content; the fit minimum is dynamic.
const MIN_ZOOM_FLOOR: f32 = 0.000_001;
const ZOOM_STEP: f32 = 1.7;
const ZOOM_ANCHOR_THRESHOLD: f32 = 0.15;
const PADDING_MAX_RATIO: f32 = 0.75;
const PADDING_MIN_RATIO: f32 = 0.15;
const PADDING_MIN_AT_ZOOM_PERCENT: f32 = 0.2;
const EDGE_AUTO_SCROLL_ZONE: f32 = 100.0;
const EDGE_AUTO_SCROLL_MAX_STEP: f32 = 15.0;
const HORIZONTAL_WHEEL_STEP: f32 = 40.0;
const RULER_LABEL_SPACING: f32 = 160.0;
const RULER_TICK_SPACING: f32 = 18.0;
const LABEL_CHARS: usize = 120;

/// App action id for every preference edit; engine history is untouched.
pub const PREFERENCE_COMMAND: &str = "ui.prefs.set";

impl Editor {
    pub(crate) fn theme(ctx: &egui::Context, dark: bool) {
        crate::kit::install_theme(ctx, dark);
    }
    pub(crate) fn fonts(ctx: &egui::Context) {
        crate::kit::install_fonts(ctx);
    }

    pub fn new(session: Session, playback: Playback, preferences: Preferences) -> Self {
        Self::with_session(
            crate::lifecycle::EditorSession::Memory(Box::new(session)),
            playback,
            preferences,
        )
    }
    fn with_session(
        session: crate::lifecycle::EditorSession,
        playback: Playback,
        preferences: Preferences,
    ) -> Self {
        let view = session.metadata().view;
        let zoom = (f64::from(DEFAULT_ZOOM) * view.zoom)
            .clamp(f64::from(MIN_ZOOM_FLOOR), f64::from(MAX_ZOOM)) as f32;
        let scroll = (view.scroll / f64::from(zoom)) as f32;
        let selection = session.selection().clone();
        let preferences_fit = preferences.preview_fit;
        let preferences_zoom = preferences.preview_zoom;
        let keymap = Keymap::resolve(&preferences.keybindings);
        let keybinding_issues = keymap.issues().to_vec();
        let keymap_source = preferences.keybindings.clone();
        Self {
            session,
            playback,
            preferences,
            preference_history: PreferenceHistory::default(),
            keymap,
            keymap_source,
            keybinding_issues,
            recording_active: false,
            overlay_open: false,
            selection,
            selection_scopes: SelectionScopes::default(),
            marquee: None,
            properties: properties::State::default(),
            effect_previews: crate::effect_previews::EffectPreviews::cpu().ok(),
            effect_textures: BTreeMap::new(),
            effect_drag: None,
            effect_drop_targets: Vec::new(),
            ripple: false,
            zoom,
            scroll,
            vertical_scroll: 0.0,
            timeline_viewport: 1000.0,
            pre_anchor_scroll: None,
            last_view: view,
            widgets: BTreeMap::new(),
            panels: BTreeMap::new(),
            frame: frame::State::default(),
            player: player::State::new(preferences_fit, preferences_zoom),
            fullscreen_commanded: false,
            snapshot: preview::SnapshotDialog::default(),
            drag: None,
            bookmark_drag: None,
            snap_indicator: None,
            error: None,
            persistence_error: None,
            configured: false,
            track_rename: None,
            track_rename_focus: false,
            track_drag: None,
            lifecycle_action: None,
            imports: None,
            import_total: 0,
            import_completed: 0,
            import_imported: 0,
            import_cancelled: false,
            import_failures: Vec::new(),
            import_notice: None,
            import_error: None,
            clock: Arc::new(SystemClock::default()),
            toast: None,
            media_selection: BTreeSet::new(),
            media_anchor: None,
            media_sizes: BTreeMap::new(),
            media_marquee: None,
            media_reveal: None,
            media_delete_confirm: None,
            scenes_open: false,
            scene_select: false,
            scene_selected: BTreeSet::new(),
            scene_confirm: false,
            scene_rename: None,
            scene_rename_focus: false,
            export: None,
            shortcuts_open: false,
            shortcut_tab: ShortcutTab::Timeline,
            shortcut_recording: None,
            shortcut_error: None,
            models: None,
            models_visible: true,
            asset_states: BTreeMap::new(),
            thumbnails: BTreeMap::new(),
            pending_imports: BTreeMap::new(),
            restoring_asset: None,
            thumbnail_textures: BTreeMap::new(),
            thumbnail_root: None,
            subtitles: None,
        }
    }
    /// Attach the engine autosave owner without duplicating its Session.
    pub fn persisted(
        autosave: engine::recovery::Autosave,
        playback: Playback,
        preferences: Preferences,
    ) -> Self {
        Self::with_session(
            crate::lifecycle::EditorSession::Persistent(Box::new(autosave)),
            playback,
            preferences,
        )
    }
    pub fn inspect(&self) -> Result<Inspection> {
        let media_order = self.media_order();
        let media_highlight = self.media_reveal.map(|reveal| reveal.asset);
        let media_delete_pending = self.media_delete_confirm.clone().unwrap_or_default();
        Ok(Inspection {
            widgets: self.widgets.values().cloned().collect(),
            view: "editor",
            playing: self.playback.playing(),
            position: self.playback.position()?,
            selection: self.selection.iter().copied().collect(),
            selection_scope: self.selection_scopes.active(&self.selection),
            zoom: self.zoom,
            scroll: self.scroll,
            min_zoom: self.min_zoom(self.timeline_viewport),
            snapping: self.preferences.snapping,
            snap_indicator: self.snap_indicator,
            ripple: self.ripple,
            preview_zoom: self.player.view.zoom,
            preview_fit: self.player.view.fit,
            preview_offset: self.player.view.offset,
            preview_fullscreen: self.player.fullscreen,
            timecode: self.current_timecode(),
            timecode_editing: self.player.timecode_edit.is_some(),
            snapshot_dialog_open: self.snapshot.is_open(),
            snapshot_saved: self.snapshot.saved.clone(),
            snapshot_error: self.snapshot.error,
            importing: self.importing(),
            import_completed: self.import_completed,
            import_total: self.import_total,
            import_imported: self.import_imported,
            import_unfinished: self.import_total.saturating_sub(self.import_completed),
            import_cancelled: self.import_cancelled,
            import_failures: self
                .import_failures
                .iter()
                .map(|failure| {
                    let name = failure.path.file_name().map_or_else(
                        || failure.path.to_string_lossy().into_owned(),
                        |name| name.to_string_lossy().into_owned(),
                    );
                    catalog::format(
                        self.preferences.locale,
                        Text::FailureDetail,
                        &[("name", &name), ("reason", &failure.reason)],
                    )
                })
                .collect(),
            import_notice: self.import_notice.clone(),
            import_error: self.import_error.clone(),
            import_toast: self.toast.as_ref().map(|toast| match &toast.detail {
                Some(detail) => format!("{} · {detail}", toast.text),
                None => toast.text.clone(),
            }),
            media_selection: self.media_selection.iter().copied().collect(),
            assets: self
                .session
                .project()
                .data()
                .assets
                .iter()
                .filter(|asset| !asset.ephemeral())
                .map(|asset| {
                    let state = self
                        .asset_states
                        .get(&asset.id())
                        .copied()
                        .unwrap_or(AssetState::Probing);
                    AssetInspection {
                        id: asset.id(),
                        state: state.code(),
                        thumbnail: self.thumbnails.contains_key(&asset.id()),
                        detail: match state {
                            AssetState::Unsupported(reason) => Some(reason.label().to_owned()),
                            _ => None,
                        },
                    }
                })
                .chain(self.pending_imports.keys().map(|id| AssetInspection {
                    id: *id,
                    state: "probing",
                    thumbnail: false,
                    detail: None,
                }))
                .collect(),
            effect_previews: self
                .effect_previews
                .as_ref()
                .map_or_else(BTreeMap::new, |previews| previews.statuses()),
            media_view: self.preferences.media_view,
            media_sort: self.preferences.media_sort,
            media_order,
            media_highlight,
            media_delete_pending,
            export_options: self.preferences.export,
            export: self
                .export
                .as_ref()
                .map(crate::export::ExportDialog::inspect),
            subtitles: self.subtitles.as_ref().map(|s| s.inspect(&self.session)),
            models: self
                .models
                .as_ref()
                .map(crate::model_manager::ModelManager::inspect),
            keybinding_issues: self.keybinding_issues.clone(),
            color_preview: self.properties.picker.as_ref().map(|p| p.color),
            eyedropper: self.properties.picker.as_ref().map(|p| p.native.state),
            shortcuts: self.shortcuts_open.then(|| {
                let platform = actions::Platform::current();
                let rows = actions::actions()
                    .iter()
                    .filter(|spec| spec.bindable())
                    .map(|spec| {
                        let chords = self.preferences.keybindings.effective_chords(spec);
                        ShortcutRow {
                            action: spec.id.to_owned(),
                            category: spec.category.label(),
                            tab: ShortcutTab::for_category(spec.category),
                            keys: chords
                                .iter()
                                .filter_map(|text| actions::Chord::parse(text).ok())
                                .map(|chord| chord.display(platform))
                                .collect(),
                            customized: self
                                .preferences
                                .keybindings
                                .is_customized
                                .contains(spec.id),
                            unbound: chords.is_empty(),
                        }
                    })
                    .collect();
                ShortcutsInspection {
                    tab: self.shortcut_tab,
                    recording: self.shortcut_recording.map(str::to_owned),
                    error: self.shortcut_error.clone(),
                    rows,
                }
            }),
        })
    }
    /// Last frame submitted by the frontend for presentation, independent of decode-ahead.
    pub fn presented_frame(&self) -> Option<Tick> {
        self.player.texture_key.map(|(_, at)| at)
    }
    /// Replace the UI timer clock in tests; production uses the system clock.
    /// Set it before raising a timed state so its deadline uses this clock.
    pub fn set_clock(&mut self, clock: Arc<dyn Clock>) {
        self.clock = clock;
    }
    /// Upload one decoded thumbnail on first use; later frames reuse the handle.
    fn thumbnail_texture(
        &mut self,
        ctx: &egui::Context,
        id: AssetId,
    ) -> Option<egui::TextureHandle> {
        if let Some(texture) = self.thumbnail_textures.get(&id) {
            return Some(texture.clone());
        }
        let thumbnail = self.thumbnails.get(&id)?;
        // One retained image pays for one CPU image and GPU upload, even when
        // several library records refer to the same content/frame.
        if let Some(texture) = self.thumbnail_textures.iter().find_map(|(other, texture)| {
            self.thumbnails
                .get(other)
                .filter(|image| image.shares_pixels(thumbnail))
                .map(|_| texture.clone())
        }) {
            self.thumbnail_textures.insert(id, texture.clone());
            return Some(texture);
        }
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [thumbnail.width as usize, thumbnail.height as usize],
            &thumbnail.pixels,
        );
        let texture = ctx.load_texture(
            format!("media.thumbnail.{}", id.0),
            image,
            egui::TextureOptions::LINEAR,
        );
        self.thumbnail_textures.insert(id, texture.clone());
        Some(texture)
    }
}
fn error_detail(error: &dyn std::error::Error) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    for _ in 0..8 {
        let Some(cause) = source else {
            break;
        };
        parts.push(cause.to_string());
        source = cause.source();
    }
    parts.join(": ")
}
fn effect_definition(params: &project::EffectParams) -> Option<&'static project::EffectDefinition> {
    let kind = match params {
        project::EffectParams::Blur { .. } => "blur",
    };
    project::effect_definitions()
        .iter()
        .find(|definition| definition.kind == kind)
}
fn effect_params_from_definition(
    definition: &project::EffectDefinition,
) -> Option<project::EffectParams> {
    match definition.kind {
        "blur" => {
            let intensity = definition
                .params
                .iter()
                .find(|parameter| parameter.key == "intensity")
                .map_or(15.0, |parameter| parameter.default);
            Some(project::EffectParams::Blur { intensity })
        }
        _ => None,
    }
}
fn effect_preview_params(definition: &project::EffectDefinition) -> Option<project::EffectParams> {
    match definition.kind {
        "blur" => {
            // A catalog tile must read as the effect at a glance, so use the
            // admitted maximum rather than the insertion default.
            let intensity = definition
                .params
                .iter()
                .find(|parameter| parameter.key == "intensity")
                .map_or(100.0, |parameter| parameter.max);
            Some(project::EffectParams::Blur { intensity })
        }
        _ => None,
    }
}
fn effect_scrub_spec(parameter: &project::EffectParamDefinition) -> ScrubSpec {
    let step = if parameter.max - parameter.min <= 1.0 {
        0.01
    } else {
        1.0
    };
    ScrubSpec {
        field: "effect_param",
        label: Text::EffectIntensity,
        min: parameter.min,
        max: parameter.max,
        step,
        pixels_per_step: 2.0,
        decimals: if step < 1.0 { 2 } else { 0 },
        suffix: "",
    }
}
fn display_label(value: &str, limit: usize) -> String {
    let mut label: String = value.chars().take(limit.saturating_add(1)).collect();
    if label.chars().count() > limit {
        label.pop();
        label.push('…');
    }
    label
}
