//! Coordinates the editor through area interfaces.
use crate::i18n;
mod assets;
mod captions;
pub(crate) mod dialogs;
mod frame;
pub use frame::{AppCommand, PreviewZoom};
pub(crate) use frame::{Context, Desk, Request};
pub(crate) mod panels;
pub(crate) use panels::player::KeyPick;
pub use panels::player::crop::{Aspect as CropAspect, Inspection as CropInspection};
pub mod properties;
mod timeline;
mod widgets;

mod state;
pub use assets::sounds::{Client, Sounds, SoundsMode, SoundsProblem, SoundsView};
use state::{
    AUDIO_GAIN_SCRUB, AssetRow, EffectReorder, MediaMarquee, MediaReveal, OPACITY_SCRUB,
    PreferenceCommand, SHORTCUT_CONTROL_HEIGHT, SHORTCUT_ROW_HEIGHT, SHORTCUT_TEXT_SIZE,
    SHORTCUT_TITLE_SIZE, SPEED_SCRUB, ScrubDrag, ScrubSpec, ScrubTarget, TEXT_SIZE_SCRUB, Toast,
};
pub use state::{
    AssetInspection, AssetState, Editor, Inspection, PreviewAxis, PreviewGuide, ShortcutRow,
    ShortcutTab, ShortcutsInspection, TextCaret,
};
pub use widgets::WidgetState;

mod timing;
use timing::{display_short_time, display_time};

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
    preferences::{Preferences, drag_panel},
    services::playback::Playback,
};
use engine::{
    Command, CreatedId, ItemParamPatch, NewItem, Session,
    jobs::{Clock, SystemClock},
};
use media::{AssetId, ContentHash, SourceAsset, StreamKind, UnsupportedReason};
use project::{BookmarkId, ItemId, ItemParams, TrackId, TrackKind};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
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

impl Editor {
    pub(crate) fn theme(ctx: &egui::Context, dark: bool) {
        crate::kit::install_theme(ctx, dark);
    }
    pub(crate) fn fonts(ctx: &egui::Context) {
        crate::kit::install_fonts(ctx);
    }
    pub(in crate::editor) fn tr(&self, key: Text) -> &'static str {
        i18n::text(self.preferences.locale, key)
    }
    pub(in crate::editor) fn accent(&self) -> Color32 {
        theme::accent(self.preferences.dark)
    }
    pub(in crate::editor) fn panel_fill(&self) -> Color32 {
        theme::panel(self.preferences.dark)
    }
    pub(in crate::editor) fn control_fill(&self) -> Color32 {
        theme::field(self.preferences.dark)
    }
    pub(in crate::editor) fn border(&self) -> Color32 {
        theme::border(self.preferences.dark)
    }
    pub(in crate::editor) fn secondary(&self) -> Color32 {
        theme::secondary(self.preferences.dark)
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
    /// Adopt the workspace's one media cache instance for timeline display
    /// reads and generation. The same instance is lent to the MCP inbox.
    pub(crate) fn set_media(
        &mut self,
        media: std::sync::Arc<std::sync::Mutex<engine::control::MediaAccess>>,
    ) {
        self.media.set_access(media);
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
        let project = session.project().data().id;
        let media = timeline::Media::new(playback.budget().clone());
        let timeline_lanes: Vec<ItemId> = preferences
            .timeline_lanes
            .get(&project)
            .into_iter()
            .flatten()
            .copied()
            .map(ItemId)
            .collect();
        let mut player = panels::player::State::new(preferences_fit, preferences_zoom);
        player.last_view = view;
        let frame = frame::State::new(&preferences);
        Self {
            session,
            playback,
            preferences,
            overlay_open: false,
            selection,
            selection_scopes: SelectionScopes::default(),
            properties: properties::State::default(),
            effect_previews: crate::effect_previews::EffectPreviews::cpu().ok(),
            effect_textures: BTreeMap::new(),
            effect_drag: None,
            frame,
            player,
            timeline: {
                let mut state = timeline::State::new(zoom, scroll);
                state.restore_lanes(timeline_lanes);
                state
            },
            media,
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
            dialogs: dialogs::State::default(),
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
            requests: Vec::new(),
        }
    }
    /// Panel rectangles from the last painted frame, for automation reads.
    pub fn panels(&self) -> &BTreeMap<&'static str, Rect> {
        &self.frame.panels
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
            widgets: self.frame.widgets.values().cloned().collect(),
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
            preview_guides: self.player.transform.guides().to_vec(),
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
            snapshot_dialog_open: self.player.snapshot.is_open(),
            snapshot_saved: self.player.snapshot.saved.clone(),
            snapshot_error: self.player.snapshot.error,
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
            export: self.dialogs.export.inspect(),
            crop: self
                .player
                .crop
                .as_ref()
                .map(panels::player::crop::Dialog::inspect),
            subtitles: self.subtitles.as_ref().map(|s| s.inspect(&self.session)),
            models: self.dialogs.models.inspect(),
            keybinding_issues: self.frame.keybinding_issues.clone(),
            color_preview: self.properties.picker.as_ref().map(|p| p.color),
            eyedropper: self.properties.picker.as_ref().map(|p| p.native.state),
            key_color_pick: self.player.key_color_pick.as_ref().map(|pick| pick.item),
            font_picker: self
                .properties
                .font_picker
                .inspect(&self.preferences.font_picker),
            text_caret: self.player.transform.text_caret(),
            shortcuts: self.frame.shortcuts_open.then(|| {
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
                    tab: self.frame.shortcut_tab,
                    recording: self.frame.shortcut_recording.map(str::to_owned),
                    error: self.frame.shortcut_error.clone(),
                    rows,
                }
            }),
        })
    }
    /// Last frame submitted by the frontend for presentation, independent of decode-ahead.
    pub fn presented_frame(&self) -> Option<Tick> {
        self.player.texture.presented()
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
/// A library tile must read as the effect at a glance, so previews use the
/// admitted maximum of the leading scalar rather than the insertion default.
fn effect_preview_instance(
    definition: &project::EffectDefinition,
) -> Option<project::EffectInstance> {
    let mut instance = definition.instance(definition.id);
    if definition.id == "blur" {
        let (_, max) = definition.param("intensity")?.scalar_range()?;
        instance.set_scalar("intensity", max).ok()?;
        return Some(instance);
    }
    None
}
fn effect_scrub_spec(parameter: &project::EffectParamDefinition) -> ScrubSpec {
    let (min, max) = match parameter.kind {
        project::EffectParamKind::Float {
            soft_min, soft_max, ..
        } => (soft_min, soft_max),
        _ => parameter.scalar_range().unwrap_or((0.0, 1.0)),
    };
    let step = if max - min <= 1.0 { 0.01 } else { 1.0 };
    ScrubSpec {
        field: "effect_param",
        label: Text::EffectIntensity,
        min,
        max,
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
