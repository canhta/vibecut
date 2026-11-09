//! Coordinates the editor through area interfaces.
use crate::i18n;
mod assets;
mod captions;
mod export;
mod frame;
pub(crate) use frame::Context;
mod models;
mod player;
pub mod properties;
mod timeline;
mod widgets;

mod state;
pub use assets::sounds::{Client, Sounds, SoundsMode, SoundsProblem, SoundsView};
use state::{
    AUDIO_GAIN_SCRUB, AssetRow, EffectReorder, MediaMarquee, MediaReveal, OPACITY_SCRUB,
    PreferenceCommand, PreferenceHistory, SHORTCUT_CONTROL_HEIGHT, SHORTCUT_ROW_HEIGHT,
    SHORTCUT_TEXT_SIZE, SHORTCUT_TITLE_SIZE, SPEED_SCRUB, ScrubDrag, ScrubSpec, ScrubTarget,
    TEXT_SIZE_SCRUB, Toast,
};
pub use state::{
    AssetInspection, AssetState, Editor, Inspection, ShortcutRow, ShortcutTab, ShortcutsInspection,
    WidgetState,
};

mod timing;
use timing::{display_short_time, display_time, drop_frame};

use crate::color_picker::{ColorTarget, EyedropperState, Picker};
use crate::selection::{KeyframeSelection, SelectionScope, SelectionScopes, shared_fields};
use crate::snapping::{SnapCandidate, SnapSource, snap_point};
use crate::{
    Error, Result,
    actions::{
        self, ActionTarget, BlockedReason, Dispatch, DispatchContext, EngineAction,
        KeybindingIssue, Keymap, UiAction,
    },
    egui::{self, Align2, Color32, FontId, Id, Rect, Vec2},
    i18n::{Locale, Text},
    import::{
        ImportEvent, ImportFailure, ImportProbe, ImportQueue, ThumbnailOutcome, ThumbnailSource,
    },
    kit as theme,
    kit::{self as icons, Icon},
    media_panel::{self, MediaKind, MediaSort, MediaSortKey, MediaSortOrder, MediaView},
    panels::{AssetsTab, ElementKind, PropertiesTab},
    playback::Playback,
    preferences::{Preferences, drag_panel},
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
    /// Inject the catalog command adapter. Production opens it lazily; tests
    /// and audits supply a recording fake.
    pub fn set_font_backend(&mut self, backend: Box<dyn properties::font_picker::Backend>) {
        self.font_backend = Some(backend);
    }
    /// Wire the My fonts tab once font import (80) supplies imported families.
    pub fn set_font_imports_available(&mut self, available: bool) {
        self.properties.font_picker.has_imports = available;
    }

    pub fn new(session: Session, playback: Playback, preferences: Preferences) -> Self {
        Self::with_session(
            crate::lifecycle::EditorSession::Memory(Box::new(session)),
            playback,
            preferences,
        )
    }
    /// A transient preview editor draws with the caller's appearance and
    /// defaults for every other preference; the shorts window owns its state.
    pub fn preview(session: Session, playback: Playback, look: crate::appearance::Look) -> Self {
        let preferences = Preferences {
            locale: look.locale,
            dark: look.dark,
            ..Preferences::default()
        };
        Self::new(session, playback, preferences)
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
        let project = session.project().data().id;
        let timeline_lanes: Vec<ItemId> = preferences
            .timeline_lanes
            .get(&project)
            .into_iter()
            .flatten()
            .copied()
            .map(ItemId)
            .collect();
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
            properties: properties::State::default(),
            effect_previews: crate::effect_previews::EffectPreviews::cpu().ok(),
            effect_textures: BTreeMap::new(),
            effect_drag: None,
            last_view: view,
            widgets: BTreeMap::new(),
            panels: BTreeMap::new(),
            frame: frame::State::default(),
            player: player::State::new(preferences_fit, preferences_zoom),
            timeline: {
                let mut state = timeline::State::new(zoom, scroll);
                state.restore_lanes(timeline_lanes);
                state
            },
            fullscreen_commanded: false,
            snapshot: preview::SnapshotDialog::default(),
            error: None,
            persistence_error: None,
            configured: false,
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
            assets: assets::State::default(),
            media_selection: BTreeSet::new(),
            media_anchor: None,
            media_sizes: BTreeMap::new(),
            media_marquee: None,
            media_reveal: None,
            media_delete_confirm: None,
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
            thumbnail_textures: std::cell::RefCell::default(),
            thumbnail_root: None,
            sound_library: None,
            sounds: None,
            sound_media_dir: None,
            subtitles: None,
            font_backend: None,
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
            expanded_lanes: self.timeline.expanded_lanes(),
            zoom: self.timeline.zoom(),
            scroll: self.timeline.scroll(),
            min_zoom: self.timeline.min_zoom(self.session.project().duration()),
            snapping: self.preferences.snapping,
            snap_indicator: self.timeline.snap_indicator(),
            bookmark_popover: self.timeline.bookmark_popover(),
            preview: self.timeline.preview_command(),
            timeline_refusal: self.timeline.refusal().map(str::to_owned),
            drop_zone: self.timeline.drop_zone_label(),
            pinned: self
                .session
                .project()
                .data()
                .scene()
                .timeline
                .main
                .items
                .iter()
                .filter(|item| item.start == Tick::ZERO)
                .map(|item| item.id)
                .collect(),
            ripple: self.preferences.ripple,
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
                    i18n::format(
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
            sounds: self.sounds.as_ref().map(|sounds| sounds.view()),
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
            font_picker: self
                .properties
                .font_picker
                .inspect(&self.preferences.font_picker),
            shortcuts: self.shortcuts_open.then(|| {
                let platform = actions::Platform::current();
                let rows = actions::actions()
                    .iter()
                    .filter(|spec| spec.bindable())
                    .map(|spec| {
                        let chords = self.preferences.keybindings.effective_chords(spec);
                        ShortcutRow {
                            action: spec.id.to_owned(),
                            label: spec.label.text(self.preferences.locale).to_owned(),
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
    fn thumbnail_texture(&self, ctx: &egui::Context, id: AssetId) -> Option<egui::TextureHandle> {
        frame::Thumbnails {
            images: &self.thumbnails,
            textures: &self.thumbnail_textures,
        }
        .texture(ctx, id)
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
        project::EffectParams::ColorGrade { .. } => "color_grade",
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
        "color_grade" => {
            let value = |key: &str| {
                definition
                    .params
                    .iter()
                    .find(|parameter| parameter.key == key)
                    .map_or(0.0, |parameter| parameter.default)
            };
            Some(project::EffectParams::ColorGrade {
                brightness: value("brightness"),
                contrast: value("contrast"),
                saturation: value("saturation"),
                hue: value("hue"),
            })
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
