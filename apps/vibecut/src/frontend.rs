use crate::selection::{SelectionScope, SelectionScopes, shared_fields};
use crate::{
    Error, Result,
    catalog::{self, Locale, Preferences, Text, drag_panel},
    egui::{self, Align2, Color32, FontId, Id, Rect, Vec2},
    icons::{self, Icon},
    import::{
        ImportEvent, ImportFailure, ImportProbe, ImportQueue, ThumbnailOutcome, ThumbnailSource,
    },
    media_panel::{self, MediaKind, MediaSort, MediaSortKey, MediaSortOrder, MediaView},
    panels::{AssetsTab, ElementKind, PropertiesTab},
    playback::Playback,
    theme,
};
use engine::{Command, CreatedId, ItemParamPatch, NewItem, Session};
use media::{AssetId, ContentHash, SourceAsset, StreamKind, UnsupportedReason};
use project::{ItemId, ItemParams, TrackId, TrackKind};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use time::{FrameRate, Rounding, SourceRange, TICKS_PER_SECOND, Tick};

const TOOLBAR_HEIGHT: f32 = 40.0;
const TOOLBAR_SPACING: f32 = 4.0;
const BUTTON_ICON: f32 = 16.0;
const ICON_BUTTON: f32 = 28.0;
const RULER_HEIGHT: f32 = 22.0;
const LABEL_WIDTH: f32 = 112.0;
const SCROLLBAR: f32 = 12.0;
const TRACK_GAP: f32 = 6.0;
const MEDIA_CARD_WIDTH: f32 = 112.0;
const MEDIA_CARD_HEIGHT: f32 = 120.0;
const MEDIA_CARD_GAP: f32 = 8.0;
const MEDIA_ROW_HEIGHT: f32 = 46.0;
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
const RULER_LABEL_SPACING: f32 = 120.0;
const RULER_TICK_SPACING: f32 = 18.0;
const SNAP_PIXELS: f32 = 10.0;
const LABEL_CHARS: usize = 120;

/// App action id for every preference edit; engine history is untouched.
pub const PREFERENCE_COMMAND: &str = "ui.prefs.set";

#[derive(Clone, Debug)]
pub struct WidgetState {
    pub id: String,
    pub enabled: bool,
    pub focused: bool,
    pub rect: Rect,
    /// Embedded Phosphor name for a button that carries an icon.
    pub icon: Option<&'static str>,
    /// Action and shortcut shown for icon-only controls.
    pub tooltip: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetState {
    Probing,
    Ready,
    Unsupported(UnsupportedReason),
}
impl AssetState {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Probing => "probing",
            Self::Ready => "ready",
            Self::Unsupported(_) => "unsupported",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetInspection {
    pub id: AssetId,
    pub state: &'static str,
    pub thumbnail: bool,
    pub detail: Option<String>,
}
#[derive(Clone, Debug)]
pub struct Inspection {
    pub widgets: Vec<WidgetState>,
    pub view: &'static str,
    pub playing: bool,
    pub position: Tick,
    pub selection: Vec<ItemId>,
    pub selection_scope: SelectionScope,
    pub zoom: f32,
    /// Persisted/logical horizontal offset in seconds at the left edge.
    pub scroll: f32,
    /// Dynamic fit minimum for the current viewport and content.
    pub min_zoom: f32,
    pub snapping: bool,
    pub ripple: bool,
    pub importing: bool,
    pub import_completed: usize,
    pub import_total: usize,
    pub import_imported: usize,
    pub import_unfinished: usize,
    pub import_cancelled: bool,
    pub import_failures: Vec<String>,
    pub import_notice: Option<String>,
    pub import_error: Option<Arc<engine::EditError>>,
    pub import_toast: Option<String>,
    pub media_selection: Vec<AssetId>,
    pub assets: Vec<AssetInspection>,
    pub effect_previews: BTreeMap<String, crate::effect_previews::EffectPreviewStatus>,
    pub media_view: MediaView,
    pub media_sort: MediaSort,
    pub media_order: Vec<AssetId>,
    pub media_highlight: Option<AssetId>,
    pub media_delete_pending: Vec<AssetId>,
    pub export: Option<crate::export::ExportInspection>,
    pub export_options: crate::export::ExportOptions,
    pub models: Option<crate::model_manager::ModelInspection>,
    pub subtitles: Option<crate::subtitles::Inspection>,
}
#[derive(Clone, Debug)]
struct PreferenceCommand {
    key: String,
    before: serde_json::Value,
    after: serde_json::Value,
}
impl PreferenceCommand {
    fn new(key: &str, before: serde_json::Value, after: serde_json::Value) -> Self {
        Self {
            key: key.to_owned(),
            before,
            after,
        }
    }
}
/// App-owned preference history, deliberately separate from the Project's
/// command history: `ui.prefs.set` edits never mutate Project JSON.
#[derive(Default)]
struct PreferenceHistory {
    undo: Vec<PreferenceCommand>,
    redo: Vec<PreferenceCommand>,
}
impl PreferenceHistory {
    fn record(&mut self, command: PreferenceCommand) {
        self.undo.push(command);
        self.redo.clear();
    }
    fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    fn undo(&mut self, preferences: &mut Preferences) -> bool {
        let Some(command) = self.undo.pop() else {
            return false;
        };
        if preferences
            .set(&command.key, command.before.clone())
            .is_err()
        {
            self.undo.push(command);
            return false;
        }
        self.redo.push(command);
        true
    }
    fn redo(&mut self, preferences: &mut Preferences) -> bool {
        let Some(command) = self.redo.pop() else {
            return false;
        };
        if preferences
            .set(&command.key, command.after.clone())
            .is_err()
        {
            self.redo.push(command);
            return false;
        }
        self.undo.push(command);
        true
    }
}
#[derive(Clone, Debug)]
struct Toast {
    text: String,
    detail: Option<String>,
    until: Instant,
}
#[derive(Clone, Debug)]
struct AssetRow {
    id: AssetId,
    name: String,
    info: String,
    brief: String,
    icon: Icon,
    state: AssetState,
    duration: Option<Tick>,
}
#[derive(Clone, Copy, Debug)]
struct MediaReveal {
    asset: AssetId,
    until: Instant,
}
#[derive(Clone, Debug)]
struct MediaMarquee {
    origin: egui::Pos2,
    before: BTreeSet<AssetId>,
    additive: bool,
    moved: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragMode {
    Move,
    TrimStart,
    TrimEnd,
}
#[derive(Clone, Copy, Debug)]
struct DragOrigin {
    item: ItemId,
    track: TrackId,
    start: Tick,
    end: Tick,
}
#[derive(Clone, Debug)]
struct DragState {
    item: ItemId,
    mode: DragMode,
    press_x: f32,
    origins: Vec<DragOrigin>,
    source_range: SourceRange,
    moved: bool,
}
#[derive(Clone, Debug)]
struct EffectReorder {
    item: ItemId,
    effect: String,
}
/// One scrub field's unit contract. Pixels per step are recorded in the app
/// README; the inventory leaves the exact sensitivity unverified.
#[derive(Clone, Copy, Debug)]
struct ScrubSpec {
    field: &'static str,
    label: Text,
    min: f64,
    max: f64,
    step: f64,
    pixels_per_step: f32,
    decimals: usize,
    suffix: &'static str,
}
impl ScrubSpec {
    fn quantize(self, value: f64) -> f64 {
        if !value.is_finite() {
            return self.min;
        }
        let steps = ((value.clamp(self.min, self.max) - self.min) / self.step).round();
        (self.min + steps * self.step).clamp(self.min, self.max)
    }
    fn format(self, value: f64) -> String {
        format!("{:.*}", self.decimals, self.quantize(value))
    }
}
const OPACITY_SCRUB: ScrubSpec = ScrubSpec {
    field: "opacity",
    label: Text::Opacity,
    min: 0.0,
    max: 1.0,
    step: 0.01,
    pixels_per_step: 4.0,
    decimals: 2,
    suffix: "",
};
const TEXT_SIZE_SCRUB: ScrubSpec = ScrubSpec {
    field: "text_size",
    label: Text::TextSize,
    min: 1.0,
    max: 4096.0,
    step: 1.0,
    pixels_per_step: 2.0,
    decimals: 0,
    suffix: "px",
};
const AUDIO_GAIN_SCRUB: ScrubSpec = ScrubSpec {
    field: "audio_gain",
    label: Text::AudioGain,
    min: -120.0,
    max: 24.0,
    step: 1.0,
    pixels_per_step: 4.0,
    decimals: 1,
    suffix: "dB",
};
#[derive(Clone, Debug, PartialEq, Eq)]
enum ScrubTarget {
    Item(&'static str),
    Effect { effect: String, key: String },
}
impl ScrubTarget {
    fn draft_key(&self) -> String {
        match self {
            Self::Item(field) => (*field).to_owned(),
            Self::Effect { effect, key } => format!("effects.{effect}.param.{key}"),
        }
    }
    fn widget_id(&self) -> String {
        match self {
            Self::Item(field) => format!("inspector.{field}.scrub"),
            Self::Effect { effect, key } => format!("effects.{effect}.param.{key}.scrub"),
        }
    }
}
#[derive(Clone, Debug)]
struct ScrubDrag {
    item: ItemId,
    target: ScrubTarget,
    start: f64,
    last: f64,
}
#[derive(Clone, Debug)]
struct TrackRow {
    id: TrackId,
    main: bool,
    audio: bool,
    display_index: usize,
    muted: bool,
    hidden: bool,
    locked: bool,
    empty: bool,
    kind: TrackKind,
    name: String,
    top: f32,
    height: f32,
}
#[derive(Clone, Debug)]
struct ItemView {
    id: ItemId,
    kind: TrackKind,
    start: Tick,
    end: Tick,
    top: f32,
    height: f32,
    locked: bool,
    label: String,
    overlay: Option<[u8; 4]>,
    source_asset: Option<AssetId>,
}
struct TimelineLayout {
    toolbar: Rect,
    ruler: Rect,
    body: Rect,
    labels: Rect,
    corner: Rect,
    horizontal: Rect,
    vertical: Rect,
}
impl TimelineLayout {
    fn new(rect: Rect) -> Self {
        let toolbar = Rect::from_min_size(
            rect.min,
            Vec2::new(rect.width(), TOOLBAR_HEIGHT.min(rect.height())),
        );
        let top = toolbar.bottom();
        let label_right = (rect.left() + LABEL_WIDTH).min(rect.right());
        let ruler_bottom = (top + RULER_HEIGHT).min(rect.bottom());
        let right = (rect.right() - SCROLLBAR).max(label_right);
        let bottom = (rect.bottom() - SCROLLBAR).max(ruler_bottom);
        let ruler = Rect::from_min_max(
            egui::pos2(label_right, top),
            egui::pos2(right, ruler_bottom),
        );
        let body = Rect::from_min_max(
            egui::pos2(ruler.left(), ruler.bottom()),
            egui::pos2(right, bottom),
        );
        let labels = Rect::from_min_max(
            egui::pos2(rect.left(), ruler.bottom()),
            egui::pos2(ruler.left(), bottom),
        );
        let corner = Rect::from_min_max(
            egui::pos2(rect.left(), top),
            egui::pos2(ruler.left(), ruler.bottom()),
        );
        let horizontal = Rect::from_min_max(
            egui::pos2(ruler.left(), bottom),
            egui::pos2(right, rect.bottom()),
        );
        let vertical = Rect::from_min_max(
            egui::pos2(right, ruler.bottom()),
            egui::pos2(rect.right(), bottom),
        );
        Self {
            toolbar,
            ruler,
            body,
            labels,
            corner,
            horizontal,
            vertical,
        }
    }
}
/// App state only. The immutable engine project is read through Session, never copied
/// into a parallel frontend project model. Every edit dispatches its registry ID.
pub struct Editor {
    pub session: crate::lifecycle::EditorSession,
    pub playback: Playback,
    pub preferences: Preferences,
    preference_history: PreferenceHistory,
    pub selection: BTreeSet<ItemId>,
    pub selection_scopes: SelectionScopes,
    marquee: Option<(egui::Pos2, BTreeSet<ItemId>, bool)>,
    property_drafts: BTreeMap<String, String>,
    property_selection: BTreeSet<ItemId>,
    /// Last inspector field that held keyboard focus. Egui surrenders focus on
    /// Escape before app UI runs, so the next frame must still defer shortcuts.
    property_focus: Option<String>,
    scrub: Option<ScrubDrag>,
    effect_previews: Option<crate::effect_previews::EffectPreviews>,
    effect_textures: BTreeMap<String, egui::TextureHandle>,
    effect_drag: Option<String>,
    effect_reorder: Option<EffectReorder>,
    effect_drop_targets: Vec<(ItemId, Rect)>,
    pub snapping: bool,
    pub ripple: bool,
    zoom: f32,
    scroll: f32,
    vertical_scroll: f32,
    /// Content viewport width used for the dynamic fit minimum.
    timeline_viewport: f32,
    /// Pixel scroll captured when zoom crossed the playhead-anchor threshold.
    pre_anchor_scroll: Option<f32>,
    last_view: project::ViewState,
    widgets: BTreeMap<String, WidgetState>,
    pub panels: BTreeMap<&'static str, Rect>,
    layout_drag: Option<[f32; 3]>,
    drag: Option<DragState>,
    snap_indicator: Option<Tick>,
    texture: Option<egui::TextureHandle>,
    texture_key: Option<(u64, Tick)>,
    texture_owner: Option<std::sync::Arc<crate::playback::Frame>>,
    error: Option<String>,
    pub(crate) persistence_error: Option<std::sync::Arc<engine::EditError>>,
    configured: bool,
    details: bool,
    rename: Option<String>,
    rename_focus: bool,
    track_rename: Option<(TrackId, String)>,
    track_rename_focus: bool,
    track_drag: Option<TrackId>,
    pub(crate) lifecycle_action: Option<crate::lifecycle::EditorAction>,
    imports: Option<ImportQueue>,
    import_total: usize,
    import_completed: usize,
    import_imported: usize,
    import_cancelled: bool,
    import_failures: Vec<ImportFailure>,
    import_notice: Option<String>,
    import_error: Option<Arc<engine::EditError>>,
    toast: Option<Toast>,
    media_selection: BTreeSet<AssetId>,
    media_anchor: Option<AssetId>,
    media_sizes: BTreeMap<AssetId, (ContentHash, Option<u64>)>,
    media_marquee: Option<MediaMarquee>,
    media_reveal: Option<MediaReveal>,
    media_delete_confirm: Option<Vec<AssetId>>,
    scenes_open: bool,
    scene_select: bool,
    scene_selected: BTreeSet<project::SceneId>,
    scene_confirm: bool,
    scene_rename: Option<(project::SceneId, String)>,
    scene_rename_focus: bool,
    pub export: Option<crate::export::ExportDialog>,
    models: Option<crate::model_manager::ModelManager>,
    asset_states: BTreeMap<AssetId, AssetState>,
    thumbnails: BTreeMap<AssetId, engine::thumbnails::RetainedThumbnail>,
    pending_imports: BTreeMap<AssetId, PathBuf>,
    restoring_asset: Option<AssetId>,
    thumbnail_textures: BTreeMap<AssetId, egui::TextureHandle>,
    thumbnail_root: Option<PathBuf>,
    pub subtitles: Option<crate::subtitles::Subtitles>,
}
impl Editor {
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
        Self {
            session,
            playback,
            preferences,
            preference_history: PreferenceHistory::default(),
            selection,
            selection_scopes: SelectionScopes::default(),
            marquee: None,
            property_drafts: BTreeMap::new(),
            property_selection: BTreeSet::new(),
            property_focus: None,
            scrub: None,
            effect_previews: crate::effect_previews::EffectPreviews::cpu().ok(),
            effect_textures: BTreeMap::new(),
            effect_drag: None,
            effect_reorder: None,
            effect_drop_targets: Vec::new(),
            snapping: true,
            ripple: false,
            zoom,
            scroll,
            vertical_scroll: 0.0,
            timeline_viewport: 1000.0,
            pre_anchor_scroll: None,
            last_view: view,
            widgets: BTreeMap::new(),
            panels: BTreeMap::new(),
            layout_drag: None,
            drag: None,
            snap_indicator: None,
            texture: None,
            texture_key: None,
            texture_owner: None,
            error: None,
            persistence_error: None,
            configured: false,
            details: false,
            rename: None,
            rename_focus: false,
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
            models: None,
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
            snapping: self.snapping,
            ripple: self.ripple,
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
        })
    }
    /// Last frame submitted by the frontend for presentation, independent of decode-ahead.
    pub fn presented_frame(&self) -> Option<Tick> {
        self.texture_key.map(|(_, at)| at)
    }
    /// Replace the media boundary in tests; production keeps the media owner.
    pub fn set_import_probe(&mut self, probe: Arc<dyn ImportProbe>) -> Result<()> {
        self.import_queue()?.set_probe(probe);
        Ok(())
    }
    /// Replace thumbnail decoding in tests; production reuses media's decoder.
    pub fn set_thumbnailer(&mut self, thumbnailer: Arc<dyn ThumbnailSource>) -> Result<()> {
        self.import_queue()?.set_thumbnailer(thumbnailer);
        Ok(())
    }
    /// Persistent content-keyed thumbnail directory. Without one thumbnails are
    /// memoized for this session only.
    pub fn set_thumbnail_root(&mut self, root: PathBuf) {
        if let Some(queue) = &mut self.imports {
            queue.set_thumbnail_root(root.clone());
        }
        self.thumbnail_root = Some(root);
    }
    /// Attach the native effect adapter to playback and thumbnail jobs, sharing
    /// playback's byte budget. Failure leaves the thumbnail composition unchanged.
    pub fn set_effects(
        &mut self,
        effects: Arc<dyn render::effects::EffectCompositor>,
    ) -> Result<()> {
        let budget = self.playback.budget().clone();
        let cache =
            engine::effect_thumbnails::EffectThumbnails::new(effects.clone(), budget.clone());
        let previews = crate::effect_previews::EffectPreviews::new(cache, budget)?;
        self.playback.set_effects(effects)?;
        self.effect_previews = Some(previews);
        self.effect_textures.clear();
        Ok(())
    }
    pub fn importing(&self) -> bool {
        self.imports.as_ref().is_some_and(ImportQueue::active)
    }
    /// Accept a native drag-drop batch through the same path as the picker.
    pub fn drop_files(&mut self, paths: Vec<PathBuf>) -> Result<()> {
        self.start_import(paths)
    }
    pub fn start_import(&mut self, paths: Vec<PathBuf>) -> Result<()> {
        if self.importing() {
            return Err(Error::Invalid("import already in progress"));
        }
        if paths.is_empty() {
            return Ok(());
        }
        let first = self.next_asset_id();
        self.import_total = paths.len();
        self.import_completed = 0;
        self.import_imported = 0;
        self.import_cancelled = false;
        self.import_failures.clear();
        self.import_notice = None;
        self.import_error = None;
        self.toast = None;
        let result = self.start_import_paths(paths, first);
        if result.is_err() {
            self.import_total = 0;
        }
        result
    }
    fn start_import_paths(&mut self, paths: Vec<PathBuf>, first: u64) -> Result<()> {
        let result = self.import_queue()?.start(paths.clone(), first);
        if matches!(result, Err(Error::Engine(engine::EditError::BudgetFull))) {
            self.thumbnail_textures.clear();
            self.thumbnails.clear();
            self.import_queue()?.evict_thumbnails();
            self.import_queue()?.start(paths, first)
        } else {
            result
        }
    }
    fn import_queue(&mut self) -> Result<&mut ImportQueue> {
        if self.imports.is_none() {
            self.imports = Some(ImportQueue::new(self.playback.budget().clone())?);
            if let Some(root) = &self.thumbnail_root
                && let Some(queue) = &mut self.imports
            {
                queue.set_thumbnail_root(root.clone());
            }
        }
        self.imports
            .as_mut()
            .ok_or(Error::Invalid("import queue unavailable"))
    }
    fn next_asset_id(&self) -> u64 {
        self.session
            .project()
            .data()
            .assets
            .iter()
            .map(|asset| asset.id().0)
            .max()
            .map_or(1, |id| id.saturating_add(1))
    }
    fn add_imported_asset(&mut self, asset: SourceAsset) -> Result<AssetId> {
        // The worker reserves this identity before publishing its probing row;
        // undoing an earlier import must not renumber later job/thumbnail events.
        let id = asset.id();
        self.session.apply(Command::AddAsset { asset })?;
        Ok(id)
    }
    fn apply_import_events(&mut self) {
        let before = self.session.project().clone();
        let polled = match self.imports.as_mut() {
            Some(queue) => queue.poll(),
            None => Ok(Vec::new()),
        };
        let events = match polled {
            Ok(events) => events,
            Err(_) if self.restoring_asset.is_some() => {
                self.finish_import();
                return;
            }
            Err(error) => {
                self.import_notice = Some(error_detail(error.as_ref()));
                self.import_error = Some(error);
                if let Some(queue) = &self.imports {
                    queue.cancel();
                }
                self.finish_import();
                return;
            }
        };
        for event in events {
            match event {
                ImportEvent::Probing { id, path } => {
                    if self.restoring_asset != Some(id) {
                        self.pending_imports.insert(id, path);
                    }
                }
                ImportEvent::Imported(imported) => {
                    self.pending_imports.remove(&imported.asset.id());
                    self.pending_imports
                        .retain(|_, path| path != imported.asset.path());
                    let path = imported.asset.path().to_owned();
                    if self.restoring_asset.is_none() {
                        self.import_completed += 1;
                    }
                    let unsupported = imported.capabilities.unsupported();
                    let thumbnail = crate::import::needs_thumbnail(&imported);
                    let adding = if self.restoring_asset == Some(imported.asset.id()) {
                        let id = imported.asset.id();
                        let matches = self.session.project().data().assets.iter().any(|a| {
                            a.id() == id && a.content_hash() == imported.asset.content_hash()
                        });
                        if matches {
                            Ok(id)
                        } else {
                            Err(Error::Invalid("restored media content changed"))
                        }
                    } else {
                        self.add_imported_asset(imported.asset)
                    };
                    match adding {
                        Ok(id) => {
                            let state = match unsupported {
                                Some(reason) => AssetState::Unsupported(reason),
                                None if thumbnail && self.restoring_asset.is_none() => {
                                    AssetState::Probing
                                }
                                None => AssetState::Ready,
                            };
                            self.asset_states.insert(id, state);
                            if self.restoring_asset.is_none() {
                                self.import_imported += 1;
                            }
                        }
                        Err(error) => {
                            if let Some(id) = self.restoring_asset {
                                self.asset_states.insert(
                                    id,
                                    AssetState::Unsupported(UnsupportedReason::OpenFailed),
                                );
                            }
                            if self.restoring_asset.is_none() {
                                self.import_failures
                                    .push(ImportFailure::local(path, error_detail(&error)));
                            }
                        }
                    }
                }
                ImportEvent::Failed(failure) => {
                    self.pending_imports.retain(|_, path| path != &failure.path);
                    if let Some(id) = self.restoring_asset {
                        self.asset_states
                            .insert(id, AssetState::Unsupported(UnsupportedReason::OpenFailed));
                    }
                    if self.restoring_asset.is_none() {
                        self.import_completed += 1;
                        self.import_failures.push(failure);
                    }
                }
                ImportEvent::Thumbnail { asset, .. }
                    if !matches!(
                        self.asset_states.get(&asset),
                        Some(AssetState::Probing | AssetState::Ready)
                    ) => {}
                ImportEvent::Thumbnail { asset, outcome } => match outcome {
                    ThumbnailOutcome::Ready(thumbnail) => {
                        while self.thumbnails.len() >= 64 {
                            if let Some((id, _)) = self.thumbnails.pop_first() {
                                self.thumbnail_textures.remove(&id);
                            }
                        }
                        self.thumbnails.insert(asset, thumbnail);
                        self.thumbnail_textures.remove(&asset);
                        self.asset_states.insert(asset, AssetState::Ready);
                    }
                    ThumbnailOutcome::Failed(error) => {
                        // The asset stays usable with a placeholder.
                        self.asset_states.insert(asset, AssetState::Ready);
                        tracing::warn!(category = "thumbnail_generation", error = %error, "thumbnail unavailable");
                    }
                },
                ImportEvent::Done { .. } if self.restoring_asset.is_some() => self.finish_import(),
                ImportEvent::Done { cancelled, error } => {
                    self.import_cancelled = cancelled;
                    if let Some(error) = error {
                        self.import_notice = Some(error_detail(error.as_ref()));
                        self.import_error = Some(error);
                    }
                    self.finish_import();
                }
            }
        }
        self.prune_asset_state();
        self.prune_media_selection();
        if !Arc::ptr_eq(&before, self.session.project()) {
            self.sync_preview();
        }
    }
    fn finish_import(&mut self) {
        self.pending_imports.clear();
        if let Some(id) = self.restoring_asset.take() {
            if self.asset_states.get(&id) == Some(&AssetState::Probing) {
                self.asset_states
                    .insert(id, AssetState::Unsupported(UnsupportedReason::OpenFailed));
            }
            return;
        }
        // Completed probes already validated these assets; cancellation or a
        // terminal thumbnail failure leaves a usable asset and a placeholder.
        for state in self.asset_states.values_mut() {
            if *state == AssetState::Probing {
                *state = AssetState::Ready;
            }
        }
        let failed_count = self.import_failures.len();
        let unfinished_count = self.import_total.saturating_sub(self.import_completed);
        let locale = self.preferences.locale;
        let imported = catalog::number(locale, self.import_imported as u64);
        let failed = catalog::number(locale, failed_count as u64);
        let unfinished = catalog::number(locale, unfinished_count as u64);
        let detail = catalog::format(
            locale,
            Text::ImportSummary,
            &[
                ("imported", &imported),
                ("failed", &failed),
                ("unfinished", &unfinished),
            ],
        );
        let text = if self.import_error.is_some() {
            self.tr(Text::ImportFailed)
        } else if self.import_cancelled {
            self.tr(Text::ImportCancelled)
        } else if failed_count > 0 || unfinished_count > 0 {
            self.tr(Text::ImportPartial)
        } else {
            self.tr(Text::ImportComplete)
        };
        self.set_toast(text, Some(detail));
        self.import_total = self.import_total.max(self.import_completed);
    }
    fn set_toast(&mut self, text: &str, detail: Option<String>) {
        self.toast = Some(Toast {
            text: text.to_owned(),
            detail,
            until: Instant::now() + std::time::Duration::from_secs(5),
        });
    }
    fn truncate_toast(&mut self) {
        if self
            .toast
            .as_ref()
            .is_some_and(|toast| Instant::now() >= toast.until)
        {
            self.toast = None;
        }
    }
    fn prune_media_selection(&mut self) {
        let existing: BTreeSet<AssetId> = self
            .session
            .project()
            .data()
            .assets
            .iter()
            .filter(|asset| !asset.ephemeral())
            .map(SourceAsset::id)
            .collect();
        self.media_selection.retain(|id| existing.contains(id));
        if self
            .media_anchor
            .is_some_and(|anchor| !existing.contains(&anchor))
        {
            self.media_anchor = None;
        }
        self.media_sizes.retain(|id, _| existing.contains(id));
        if self
            .media_reveal
            .is_some_and(|reveal| !existing.contains(&reveal.asset))
        {
            self.media_reveal = None;
        }
        if let Some(pending) = &mut self.media_delete_confirm {
            pending.retain(|id| existing.contains(id));
            if pending.is_empty() {
                self.media_delete_confirm = None;
            }
        }
    }
    fn media_size_map(&self) -> BTreeMap<AssetId, Option<u64>> {
        self.media_sizes
            .iter()
            .map(|(id, (_, size))| (*id, *size))
            .collect()
    }
    /// File size is display metadata; stat each asset once per content identity.
    fn refresh_media_sizes(&mut self) {
        let assets = &self.session.project().data().assets;
        let mut next = BTreeMap::new();
        for asset in assets.iter().filter(|asset| !asset.ephemeral()) {
            let hash = asset.content_hash();
            let size = match self.media_sizes.get(&asset.id()) {
                Some((cached, Some(size))) if *cached == hash => Some(*size),
                Some((cached, None)) if *cached == hash => media_panel::file_size(asset.path()),
                _ => media_panel::file_size(asset.path()),
            };
            next.insert(asset.id(), (hash, size));
        }
        self.media_sizes = next;
    }
    fn media_order(&self) -> Vec<AssetId> {
        media_panel::ordered_assets(
            &self.session.project().data().assets,
            self.preferences.media_sort,
            &self.media_size_map(),
        )
    }
    fn prune_asset_state(&mut self) {
        let existing: BTreeSet<AssetId> = self
            .session
            .project()
            .data()
            .assets
            .iter()
            .map(SourceAsset::id)
            .collect();
        self.asset_states.retain(|id, _| existing.contains(id));
        self.thumbnails.retain(|id, _| existing.contains(id));
        self.thumbnail_textures
            .retain(|id, _| existing.contains(id));
        if let Some(queue) = &mut self.imports {
            queue.retain_assets(&self.session.project().data().assets);
        }
    }
    fn restore_asset_state(&mut self) {
        if self.importing() {
            return;
        }
        let asset = self
            .session
            .project()
            .data()
            .assets
            .iter()
            .find(|a| !a.ephemeral() && !self.asset_states.contains_key(&a.id()))
            .cloned();
        if let Some(asset) = asset {
            let id = asset.id();
            self.asset_states.insert(id, AssetState::Probing);
            self.restoring_asset = Some(id);
            let result = self.start_import_paths(vec![asset.path().to_owned()], id.0);
            if result.is_err() {
                self.restoring_asset = None;
                self.asset_states
                    .insert(id, AssetState::Unsupported(UnsupportedReason::OpenFailed));
            }
        }
    }
    /// Replace the media selection; tests and agents use the same state as clicks.
    pub fn select_media(&mut self, ids: impl IntoIterator<Item = AssetId>) {
        self.media_selection = ids.into_iter().collect();
        self.media_anchor = self.media_selection.iter().next().copied();
        self.session.end_merge();
    }
    fn select_asset(&mut self, id: AssetId, modifiers: egui::Modifiers) {
        let ids = self.media_order();
        crate::selection::click(
            &mut self.media_selection,
            &mut self.media_anchor,
            id,
            modifiers.command || modifiers.ctrl,
            modifiers.shift.then_some(ids.as_slice()),
        );
        self.session.end_merge();
    }
    /// App action `media.reveal`: switch to Media, scroll and highlight for 1.5 s.
    /// It changes presentation state only; the project snapshot is untouched.
    pub fn reveal_media(&mut self, asset: AssetId) -> Result<()> {
        if !self
            .session
            .project()
            .data()
            .assets
            .iter()
            .any(|candidate| candidate.id() == asset && !candidate.ephemeral())
        {
            return Err(Error::Engine(engine::EditError::NoAsset(asset)));
        }
        if self.preferences.assets_tab != AssetsTab::Media {
            self.preferences
                .set("assets_tab", serde_json::to_value(AssetsTab::Media)?)?;
        }
        self.media_reveal = Some(MediaReveal {
            asset,
            until: Instant::now() + MEDIA_REVEAL,
        });
        self.session.end_merge();
        Ok(())
    }
    fn request_media_delete(&mut self, ids: Vec<AssetId>) {
        let existing: BTreeSet<AssetId> = self
            .session
            .project()
            .data()
            .assets
            .iter()
            .filter(|asset| !asset.ephemeral())
            .map(SourceAsset::id)
            .collect();
        let mut ids: Vec<AssetId> = ids.into_iter().filter(|id| existing.contains(id)).collect();
        ids.sort_unstable();
        ids.dedup();
        if !ids.is_empty() {
            self.media_delete_confirm = Some(ids);
        }
    }
    fn confirm_media_delete(&mut self) -> Result<()> {
        let Some(ids) = self.media_delete_confirm.take() else {
            return Ok(());
        };
        let count = ids.len() as u64;
        let before = self.session.project().clone();
        match self
            .session
            .execute("media.remove", serde_json::json!({ "assets": ids }))
        {
            Ok(_) => {
                self.media_selection.clear();
                self.media_anchor = None;
                self.media_reveal = None;
                self.prune_media_selection();
                if !Arc::ptr_eq(&before, self.session.project()) {
                    self.sync_preview();
                }
                let text = catalog::format_count(
                    self.preferences.locale,
                    count,
                    Text::MediaDeletedOne,
                    Text::MediaDeletedOther,
                );
                self.set_toast(&text, None);
                Ok(())
            }
            Err(error) => {
                let detail = error_detail(&error);
                self.import_notice = Some(detail.clone());
                self.set_toast(self.tr(Text::DeleteMediaFailed), Some(detail));
                Ok(())
            }
        }
    }
    /// Insert selected assets in library order; engine owns track and overlap policy.
    fn insert_selected_assets(&mut self) {
        let selected: Vec<SourceAsset> = self
            .session
            .project()
            .data()
            .assets
            .iter()
            .filter(|asset| !asset.ephemeral())
            .filter(|asset| self.media_selection.contains(&asset.id()))
            .cloned()
            .collect();
        if selected.is_empty() {
            return;
        }
        let at = match self.playback.position() {
            Ok(at) => at,
            Err(error) => {
                self.set_toast(self.tr(Text::InsertFailed), Some(error_detail(&error)));
                return;
            }
        };
        let unsupported: BTreeSet<AssetId> = selected
            .iter()
            .filter(|asset| self.asset_states.get(&asset.id()) != Some(&AssetState::Ready))
            .map(SourceAsset::id)
            .collect();
        let result = self.session.transaction(|session| {
            let mut created = Vec::new();
            let mut video_at = at;
            let mut audio_at = at;
            for asset in &selected {
                if unsupported.contains(&asset.id()) {
                    return Err(engine::EditError::Unsupported(
                        "media codec is unsupported in this build",
                    ));
                }
                let kind = asset_kind(asset)?;
                let duration = asset
                    .probe()
                    .duration
                    .filter(|duration| *duration > Tick::ZERO)
                    .ok_or(engine::EditError::Unsupported(
                        "media duration is unavailable",
                    ))?;
                let item = NewItem {
                    animations: Default::default(),
                    source_asset: Some(asset.id()),
                    source_range: SourceRange::new(Tick::ZERO, duration)?,
                    start: if kind == TrackKind::Video {
                        video_at
                    } else {
                        audio_at
                    },
                    content: match kind {
                        TrackKind::Video => ItemParams::Video(project::VideoParams {
                            source_audio_enabled: true,
                        }),
                        _ => ItemParams::Audio(project::AudioParams {
                            gain_db: 0.0,
                            muted: false,
                        }),
                    },
                };
                let outcome = session.apply(Command::InsertItem {
                    placement: engine::Placement::Auto,
                    item,
                })?;
                let placed = outcome
                    .placements
                    .first()
                    .ok_or(engine::EditError::Invalid("item placement"))?;
                let end = session
                    .project()
                    .data()
                    .scene()
                    .timeline
                    .iter()
                    .find(|track| track.id == placed.track)
                    .and_then(|track| track.items.iter().find(|item| item.id == placed.item))
                    .ok_or(engine::EditError::NoItem(placed.item))?
                    .end()?;
                if kind == TrackKind::Video {
                    video_at = end;
                } else {
                    audio_at = end;
                }
                created.extend(outcome.created.into_iter().filter_map(|id| match id {
                    CreatedId::Item(item) => Some(item),
                    _ => None,
                }));
            }
            Ok(created)
        });
        match result {
            Ok(created) => {
                self.selection = created.into_iter().collect();
                self.import_notice = None;
                self.session.end_merge();
            }
            Err(error) => {
                let detail = error_detail(&error);
                self.import_notice = Some(detail.clone());
                self.set_toast(self.tr(Text::InsertFailed), Some(detail));
            }
        }
    }
    fn insert_text(&mut self) {
        let at = match self.playback.position() {
            Ok(at) => at,
            Err(error) => {
                self.set_toast(self.tr(Text::TextInsertFailed), Some(error_detail(&error)));
                return;
            }
        };
        let content = self.tr(Text::TextItem).to_owned();
        let result = self.session.transaction(|session| {
            let duration = Tick(5 * TICKS_PER_SECOND);
            let outcome = session.apply(Command::InsertItem {
                placement: engine::Placement::Auto,
                item: NewItem {
                    animations: Default::default(),
                    source_asset: None,
                    source_range: SourceRange::new(Tick::ZERO, duration)?,
                    start: at,
                    content: ItemParams::Text(project::TextParams::new(content)),
                },
            })?;
            outcome
                .created
                .into_iter()
                .find_map(|id| match id {
                    CreatedId::Item(item) => Some(item),
                    _ => None,
                })
                .ok_or(engine::EditError::Unsupported("text item allocation"))
        });
        match result {
            Ok(item) => {
                self.selection = [item].into_iter().collect();
                self.selection_scopes = SelectionScopes::default();
                self.property_drafts.clear();
            }
            Err(error) => {
                let detail = error_detail(&error);
                self.import_notice = Some(detail.clone());
                self.set_toast(self.tr(Text::TextInsertFailed), Some(detail));
            }
        }
    }
    fn tr(&self, key: Text) -> &'static str {
        catalog::text(self.preferences.locale, key)
    }
    fn remember(&mut self, id: impl Into<String>, response: &egui::Response) {
        self.remember_with_icon(id, None, response);
    }
    fn remember_with_icon(
        &mut self,
        id: impl Into<String>,
        icon: Option<&'static str>,
        response: &egui::Response,
    ) {
        let id = id.into();
        response
            .ctx
            .accesskit_node_builder(response.id, |node| node.set_author_id(&id));
        self.widgets.insert(
            id.clone(),
            WidgetState {
                id,
                enabled: response.enabled(),
                focused: response.has_focus(),
                rect: response.rect,
                icon,
                tooltip: None,
            },
        );
    }
    fn button(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        icon: Icon,
        key: Text,
        enabled: bool,
    ) -> bool {
        let color = if enabled {
            self.text_color()
        } else {
            theme::disabled(self.preferences.dark)
        };
        let image = icons::tinted(icon, BUTTON_ICON, color);
        let label = egui::RichText::new(self.tr(key)).color(color);
        let response = ui
            .push_id(id, |ui| {
                ui.add_enabled(
                    enabled,
                    egui::Button::image_and_text(image, label).min_size(Vec2::new(28.0, 26.0)),
                )
            })
            .inner;
        self.remember_with_icon(id, Some(icon.name()), &response);
        response.clicked()
    }
    /// Text-only is reserved for the properties tabs by the iconography contract.
    fn text_button(&mut self, ui: &mut egui::Ui, id: &'static str, key: Text) -> bool {
        let response = ui.push_id(id, |ui| ui.button(self.tr(key))).inner;
        self.remember(id, &response);
        response.clicked()
    }
    /// A property section whose open state lives in the preference store, with
    /// the same stable automation id across locale and layout changes.
    fn section<R>(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        key: &'static str,
        label: Text,
        default_open: bool,
        body: impl FnOnce(&mut Self, &mut egui::Ui) -> R,
    ) -> Option<R> {
        let state_id = ui.make_persistent_id(("preferences.section", key));
        let stored = self.preferences.section_open(key, default_open);
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            state_id,
            default_open,
        );
        if state.is_open() != stored {
            state.set_open(stored);
            state.store(ui.ctx());
        }
        let label = self.tr(label);
        let mut clicked = false;
        let mut header = state.show_header(ui, |ui| {
            let response = ui.add(egui::Label::new(label).sense(egui::Sense::click()));
            clicked = response.clicked();
        });
        if clicked {
            header.toggle();
        }
        let open = header.is_open();
        let (toggle, _, body_response) = header.body(|ui| body(self, ui));
        if open != stored
            && let Err(error) = self.set_section_open(key, open)
        {
            self.error = Some(error.to_string());
        }
        toggle
            .ctx
            .accesskit_node_builder(toggle.id, |node| node.set_author_id(id));
        self.remember(id, &toggle);
        body_response.map(|response| response.inner)
    }
    #[allow(clippy::too_many_arguments)]
    fn icon_only(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        icon: Icon,
        label: &str,
        shortcut: Option<&str>,
        selected: Option<bool>,
        enabled: bool,
    ) -> bool {
        let locale = self.preferences.locale;
        let tooltip = shortcut.map_or_else(
            || label.to_owned(),
            |key| {
                catalog::format(
                    locale,
                    Text::ActionShortcut,
                    &[("action", label), ("shortcut", key)],
                )
            },
        );
        let is_selected = selected.unwrap_or(false);
        let (rect, response) = ui
            .push_id(id, |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    ui.allocate_exact_size(Vec2::splat(ICON_BUTTON), egui::Sense::click())
                })
                .inner
            })
            .inner;
        let color = if !enabled {
            theme::disabled(self.preferences.dark)
        } else if is_selected {
            self.accent()
        } else {
            self.text_color()
        };
        if enabled && (response.hovered() || response.has_focus()) {
            ui.painter()
                .rect_filled(rect, 4.0, theme::field(self.preferences.dark));
        }
        icons::tinted(icon, BUTTON_ICON, color).paint_at(
            ui,
            Rect::from_center_size(rect.center(), Vec2::splat(BUTTON_ICON)),
        );
        response.widget_info(|| {
            if let Some(selected) = selected {
                egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, selected, &tooltip)
            } else {
                egui::WidgetInfo::labeled(egui::Role::Button, enabled, &tooltip)
            }
        });
        let response = response.on_hover_text(&tooltip);
        self.remember_with_icon(id, Some(icon.name()), &response);
        if let Some(state) = self.widgets.get_mut(id) {
            state.tooltip = Some(tooltip);
        }
        enabled && response.clicked()
    }
    /// All control inputs (including tests/agents) use the same action path.
    /// Project edits are engine registry calls; transport/preferences are app state.
    pub fn activate(&mut self, id: &str) -> Result<()> {
        let flushed = if let Some(subtitles) = &mut self.subtitles {
            if matches!(
                id,
                "subtitles.dismiss"
                    | "subtitles.cancel"
                    | "subtitles.consent.approve"
                    | "subtitles.consent.deny"
            ) {
                false
            } else {
                subtitles.flush_drafts(&mut self.session)?
            }
        } else {
            false
        };
        if flushed {
            self.sync_preview();
        }
        self.session.set_selection(self.selection.clone())?;
        self.remember_view()?;
        let at = self.playback.position()?;
        let duration = self.session.project().duration();
        let playing = self.playback.playing();
        let before = self.session.project().clone();
        match id {
            action if action.starts_with("subtitles.") => {
                self.prepare_subtitles()?;
                let mut options = self.preferences.subtitles.clone();
                if let Some(panel) = &mut self.subtitles {
                    panel.activate(
                        &mut self.session,
                        &mut options,
                        self.selection.iter().next().copied(),
                        at,
                        action,
                    )?;
                    let seek = panel.requested_seek.take();
                    let model = panel.requested_model.take();
                    if let Some(at) = seek {
                        self.seek(at)?;
                    }
                    if let Some(id) = model {
                        self.open_model(&id)?;
                    }
                }
                if options != self.preferences.subtitles {
                    self.set_preference("subtitles", serde_json::to_value(options)?)?;
                }
            }
            "scene.open" => self.scenes_open = true,
            "scene.close" => {
                self.scenes_open = false;
                self.scene_confirm = false;
                self.scene_rename = None;
            }
            "scene.create" => {
                let count = self.session.project().data().scenes.iter().count();
                let number = catalog::number(self.preferences.locale, (count + 1) as u64);
                let name = catalog::format(
                    self.preferences.locale,
                    Text::SceneDefaultName,
                    &[("n", &number)],
                );
                let created = self
                    .session
                    .execute("scene.create", serde_json::json!({"name":name}))?;
                if let Some(CreatedId::Scene(scene)) = created.created.first().copied() {
                    self.cancel_interaction();
                    self.session
                        .execute("scene.switch", serde_json::json!({"scene":scene}))?;
                    self.selection.clear();
                    self.selection_scopes = SelectionScopes::default();
                    self.scene_select = false;
                    self.scene_selected.clear();
                    self.scene_confirm = false;
                    self.scene_rename = None;
                }
            }
            "scene.select" => {
                self.scene_select = !self.scene_select;
                self.scene_selected.clear();
            }
            "scene.delete" => self.scene_confirm = !self.scene_selected.is_empty(),
            "scene.cancel_delete" => self.scene_confirm = false,
            "scene.confirm_delete" => {
                self.session.execute(
                    "scene.delete",
                    serde_json::json!({"scenes":self.scene_selected}),
                )?;
                self.scene_selected.clear();
                self.scene_select = false;
                self.scene_confirm = false;
                self.selection.clear();
                self.selection_scopes = SelectionScopes::default();
            }
            rename if rename.starts_with("scene.rename.") => {
                let scene = project::SceneId(
                    rename
                        .trim_start_matches("scene.rename.")
                        .parse()
                        .map_err(|_| Error::Unsupported("scene identity"))?,
                );
                let name = self
                    .session
                    .project()
                    .data()
                    .scenes
                    .get(scene)
                    .ok_or(Error::Unsupported("scene not found"))?
                    .name
                    .as_str()
                    .to_owned();
                self.scene_rename = Some((scene, name));
                self.scene_rename_focus = true;
            }
            row if row.starts_with("scene.row.") => {
                let scene = project::SceneId(
                    row.trim_start_matches("scene.row.")
                        .parse()
                        .map_err(|_| Error::Unsupported("scene identity"))?,
                );
                let target = self
                    .session
                    .project()
                    .data()
                    .scenes
                    .get(scene)
                    .ok_or(Error::Unsupported("scene not found"))?;
                if self.scene_select {
                    if !target.is_main && !self.scene_selected.remove(&scene) {
                        self.scene_selected.insert(scene);
                    }
                } else {
                    self.cancel_interaction();
                    self.session
                        .execute("scene.switch", serde_json::json!({"scene":scene}))?;
                    self.selection.clear();
                    self.selection_scopes = SelectionScopes::default();
                }
            }
            "file.export" => {
                if self.export.is_none() {
                    self.export = Some(crate::export::ExportDialog::new(
                        self.playback.budget().clone(),
                        self.playback.render_state()?,
                    )?);
                }
                if let Some(dialog) = &mut self.export {
                    dialog.show(self.session.project());
                }
            }
            "export.run" => {
                if self.export.is_none() {
                    self.export = Some(crate::export::ExportDialog::new(
                        self.playback.budget().clone(),
                        self.playback.render_state()?,
                    )?);
                }
                let dialog = self
                    .export
                    .as_mut()
                    .ok_or(Error::Unsupported("export dialog"))?;
                dialog.show(self.session.project());
                dialog.start(self.session.project().clone(), self.preferences.export)?;
            }
            "export.retry" => {
                let dialog = self
                    .export
                    .as_mut()
                    .ok_or(Error::Unsupported("export dialog"))?;
                dialog.retry(self.session.project().clone())?;
            }
            "project.save" => self.lifecycle_action = Some(crate::lifecycle::EditorAction::Save),
            "project.save_as" => {
                self.lifecycle_action = Some(crate::lifecycle::EditorAction::SaveAs)
            }
            "project.back" => self.lifecycle_action = Some(crate::lifecycle::EditorAction::Leave),
            "project.rename" => {
                self.rename = Some(self.session.project().data().name.as_str().to_owned());
                self.rename_focus = true;
            }
            "file.import" => {
                if self.importing() {
                    self.import_notice = Some(self.tr(Text::ImportBusy).to_owned());
                } else {
                    self.lifecycle_action = Some(crate::lifecycle::EditorAction::ImportMedia);
                }
            }
            "media.insert" => self.insert_selected_assets(),
            "text.insert" => self.insert_text(),
            "media.view" => {
                let view = match self.preferences.media_view {
                    MediaView::Grid => MediaView::List,
                    MediaView::List => MediaView::Grid,
                };
                self.dispatch(
                    PREFERENCE_COMMAND,
                    serde_json::json!({"key": "media_view", "value": view}),
                )?;
            }
            "media.sort" => {
                let next = media_panel::next_sort(
                    self.preferences.media_sort,
                    self.preferences.media_sort.key,
                );
                self.dispatch(
                    PREFERENCE_COMMAND,
                    serde_json::json!({"key": "media_sort", "value": next}),
                )?;
            }
            row if row.starts_with("media.sort.") => {
                let key = MediaSortKey::from_id(row.trim_start_matches("media.sort."))
                    .ok_or(Error::Unsupported("media sort key"))?;
                let next = media_panel::next_sort(self.preferences.media_sort, key);
                self.dispatch(
                    PREFERENCE_COMMAND,
                    serde_json::json!({"key": "media_sort", "value": next}),
                )?;
            }
            "media.delete" => {
                self.request_media_delete(self.media_selection.iter().copied().collect())
            }
            "media.cancel_delete" => self.media_delete_confirm = None,
            "media.confirm_delete" => self.confirm_media_delete()?,
            row if row.starts_with("media.reveal.") => {
                let asset = AssetId(
                    row.trim_start_matches("media.reveal.")
                        .parse()
                        .map_err(|_| Error::Unsupported("media identity"))?,
                );
                self.reveal_media(asset)?;
            }
            "import.cancel" => {
                if let Some(queue) = &self.imports {
                    queue.cancel();
                }
            }
            "transport.play" => self.playback.seek(
                if at >= duration { Tick::ZERO } else { at },
                !playing,
                self.session.project().clone(),
            )?,
            "transport.pause" => self
                .playback
                .seek(at, false, self.session.project().clone())?,
            "transport.back" => self.seek((at - Tick(TICKS_PER_SECOND)).max(Tick::ZERO))?,
            "transport.forward" => self.seek((at + Tick(TICKS_PER_SECOND)).min(duration))?,
            "transport.frame_back" => self.step_frames(-1)?,
            "transport.frame_forward" => self.step_frames(1)?,
            "transport.start" => self.seek(Tick::ZERO)?,
            "transport.end" => self.seek(duration)?,
            "edit.undo" | "edit.redo" => {
                let preference_changed = if id == "edit.undo" {
                    self.preference_history.undo(&mut self.preferences)
                } else {
                    self.preference_history.redo(&mut self.preferences)
                };
                if !preference_changed {
                    self.session.execute(
                        if id == "edit.undo" { "undo" } else { "redo" },
                        serde_json::json!({}),
                    )?;
                    self.selection = self.session.selection().clone();
                    self.selection_scopes = SelectionScopes::default();
                    self.property_drafts.clear();
                }
            }
            "timeline.split" => self.split_at_playhead()?,
            "timeline.delete" => self.delete_selected(self.ripple)?,
            "timeline.add_track" => self.add_track(),
            "timeline.ripple_delete" => self.delete_selected(true)?,
            "timeline.select_all" => self.select_all(),
            "timeline.deselect_all" => {
                self.selection.clear();
                self.selection_scopes = SelectionScopes::default();
            }
            "selection.escape" => self.cancel_interaction(),
            "timeline.snap" => self.snapping = !self.snapping,
            "timeline.ripple" => self.ripple = !self.ripple,
            "timeline.zoom_in" => self.zoom_by(ZOOM_STEP),
            "timeline.zoom_out" => self.zoom_by(1.0 / ZOOM_STEP),
            "appearance.theme" => {
                let after = !self.preferences.dark;
                self.dispatch(
                    PREFERENCE_COMMAND,
                    serde_json::json!({"key": "dark", "value": after}),
                )?;
            }
            "appearance.locale" => {
                let after = if self.preferences.locale == Locale::En {
                    Locale::Vi
                } else {
                    Locale::En
                };
                self.dispatch(
                    PREFERENCE_COMMAND,
                    serde_json::json!({"key": "locale", "value": after}),
                )?;
            }
            "layout.reset" => {
                self.dispatch(
                    PREFERENCE_COMMAND,
                    serde_json::json!({
                        "key": "panels",
                        "value": Preferences::default().panels,
                    }),
                )?;
            }
            "models.open" => self.open_models()?,
            "preview.retry" => {
                self.playback.clear()?;
                self.error = None;
            }
            "inspector.project" => self.details = false,
            "inspector.details" => self.details = true,
            id if id.starts_with("timeline.item.") => {
                let raw = id.trim_start_matches("timeline.item.");
                let parsed = raw
                    .parse::<u64>()
                    .map_err(|_| Error::Unsupported("widget action"))?;
                let item = ItemId(parsed);
                if !self
                    .session
                    .project()
                    .data()
                    .scene()
                    .timeline
                    .iter()
                    .any(|t| t.items.iter().any(|i| i.id == item))
                {
                    return Err(Error::Engine(engine::EditError::NoItem(item)));
                }
                self.select_single(item);
            }
            _ => return Err(Error::Unsupported("widget action")),
        }
        self.prune_media_selection();
        if !Arc::ptr_eq(&before, self.session.project()) {
            self.sync_preview();
        }
        self.remember_view()?;
        Ok(())
    }
    /// Restart preview/audio work against the current immutable project snapshot.
    /// Old generations are fenced by `Playback::seek`; the position is clamped when
    /// an edit shortens the timeline and the transport state is preserved when able.
    fn sync_preview(&mut self) {
        let project = self.session.project().clone();
        let duration = project.duration();
        let at = self
            .playback
            .position()
            .unwrap_or(Tick::ZERO)
            .clamp(Tick::ZERO, duration);
        let playing = self.playback.playing();
        if self.playback.seek(at, playing, project.clone()).is_err()
            && let Err(error) = self.playback.seek(at, false, project)
        {
            self.error = Some(error.to_string());
        }
    }
    fn seek(&mut self, at: Tick) -> Result<()> {
        self.playback
            .seek(at, self.playback.playing(), self.session.project().clone())?;
        self.remember_view()
    }
    fn remember_view(&mut self) -> Result<()> {
        if self.session.transaction_active() {
            return Ok(());
        }
        let stored = self.session.metadata().view;
        // Other adapters can issue the engine view command between frontend actions.
        if stored != self.last_view {
            self.pre_anchor_scroll = None;
            self.zoom = (f64::from(DEFAULT_ZOOM) * stored.zoom)
                .clamp(f64::from(MIN_ZOOM_FLOOR), f64::from(MAX_ZOOM))
                as f32;
            self.scroll = (stored.scroll / f64::from(self.zoom)) as f32;
            self.playback.seek(
                stored
                    .playhead
                    .clamp(Tick::ZERO, self.session.project().duration()),
                self.playback.playing(),
                self.session.project().clone(),
            )?;
            self.last_view = stored;
        }
        let view = project::ViewState {
            zoom: f64::from(self.zoom) / f64::from(DEFAULT_ZOOM),
            scroll: (f64::from(self.scroll) * f64::from(self.zoom)).clamp(0.0, 1e9),
            playhead: self.playback.position()?,
        };
        if self.session.metadata().view != view {
            self.session.execute(
                "project.set_view_state",
                serde_json::json!({"zoom":view.zoom,"scroll":view.scroll,"playhead":view.playhead}),
            )?;
        }
        self.last_view = view;
        Ok(())
    }
    fn seek_to(&mut self, at: Tick) {
        let at = at.clamp(Tick::ZERO, self.session.project().duration());
        if let Err(error) = self.seek(at) {
            self.error = Some(error.to_string());
        }
    }
    fn step_frames(&mut self, frames: i64) -> Result<()> {
        let frame = self.frame_ticks();
        let at = self.playback.position()?;
        self.seek(
            (at + Tick(frame.0.saturating_mul(frames)))
                .clamp(Tick::ZERO, self.session.project().duration()),
        )
    }
    fn frame_ticks(&self) -> Tick {
        self.session
            .project()
            .data()
            .canvas
            .fps
            .frame_duration()
            .ok()
            .filter(|frame| *frame > Tick::ZERO)
            .unwrap_or(Tick(1))
    }
    fn min_zoom(&self, viewport_width: f32) -> f32 {
        fit_zoom(viewport_width, self.session.project().duration().seconds())
    }
    fn apply_zoom(&mut self, target: f32) {
        let min = self.min_zoom(self.timeline_viewport);
        let target = target.clamp(min, MAX_ZOOM);
        if (target - self.zoom).abs() <= f32::EPSILON {
            self.zoom = target;
            return;
        }
        let playhead = self.playback.position().unwrap_or(Tick::ZERO).seconds() as f32;
        let (scroll, pre_anchor) = anchored_scroll(
            self.zoom,
            target,
            min,
            self.scroll,
            playhead,
            self.pre_anchor_scroll,
        );
        self.zoom = target;
        self.scroll = scroll;
        self.pre_anchor_scroll = pre_anchor;
    }
    fn zoom_by(&mut self, factor: f32) {
        let target = (self.zoom * factor).clamp(MIN_ZOOM_FLOOR, MAX_ZOOM);
        self.apply_zoom(target);
    }
    fn scroll_bounds(&self, layout: &TimelineLayout, duration: Tick) -> (f32, f32) {
        let view = (layout.body.width() / self.zoom).max(0.001);
        let min = self.min_zoom(layout.body.width());
        let max = max_scroll_seconds(duration.seconds(), layout.body.width(), self.zoom, min);
        (view, max)
    }
    fn action(&mut self, id: &str) {
        if let Err(error) = self.activate(id) {
            self.error = Some(error.to_string());
        }
    }
    /// App action dispatcher shared by UI and agents. Preference edits use
    /// `ui.prefs.set` with `{"key": ..., "value": ...}` typed params.
    pub fn dispatch(&mut self, id: &str, params: serde_json::Value) -> Result<()> {
        if id == PREFERENCE_COMMAND {
            let key = params
                .get("key")
                .and_then(serde_json::Value::as_str)
                .ok_or(Error::Invalid("preference key"))?;
            let value = params
                .get("value")
                .cloned()
                .ok_or(Error::Invalid("preference value"))?;
            return self.set_preference(key, value);
        }
        if id == "media.reveal" {
            let asset = params
                .get("asset")
                .and_then(serde_json::Value::as_u64)
                .ok_or(Error::Invalid("media identity"))?;
            return self.reveal_media(AssetId(asset));
        }
        self.activate(id)
    }
    /// One command (`ui.prefs.set`) for every user-visible preference edit.
    /// App history is separate from Project history and never changes Project JSON.
    pub fn set_preference(&mut self, key: &str, value: serde_json::Value) -> Result<()> {
        let before = self.preferences.get(key)?;
        self.preferences.set(key, value)?;
        let after = self.preferences.get(key)?;
        if before != after {
            self.preference_history
                .record(PreferenceCommand::new(key, before, after));
        }
        Ok(())
    }
    /// Section keys are dynamic; the whole open/closed map is one history entry.
    pub fn set_section_open(&mut self, key: &str, open: bool) -> Result<()> {
        let mut sections = self.preferences.sections.clone();
        sections.insert(key.to_owned(), open);
        self.dispatch(
            PREFERENCE_COMMAND,
            serde_json::json!({"key": "sections", "value": sections}),
        )
    }
    fn record_layout_change(&mut self, before: [f32; 3]) {
        let after = self.preferences.panels;
        self.preferences.panels = before;
        if let Err(error) = self.dispatch(
            PREFERENCE_COMMAND,
            serde_json::json!({"key": "panels", "value": after}),
        ) {
            self.error = Some(error.to_string());
        }
    }
    fn select_single(&mut self, item: ItemId) {
        self.selection.clear();
        self.selection.insert(item);
        self.selection_changed();
    }
    fn select_item(&mut self, item: ItemId, modifiers: egui::Modifiers) {
        crate::selection::click(
            &mut self.selection,
            &mut None,
            item,
            modifiers.command || modifiers.ctrl || modifiers.shift,
            None,
        );
        self.selection_changed();
    }
    fn select_all(&mut self) {
        self.selection = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter().map(|item| item.id))
            .collect();
        self.selection_changed();
    }
    fn selection_changed(&mut self) {
        if self.scrub.is_some() {
            self.finish_scrub(false);
        }
        self.selection_scopes = SelectionScopes::default();
        self.property_drafts.clear();
        if let Err(e) = self.session.set_selection(self.selection.clone()) {
            self.error = Some(e.to_string());
        }
    }
    fn prune_selection(&mut self) {
        let existing: BTreeSet<ItemId> = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter().map(|item| item.id))
            .collect();
        self.selection.retain(|id| existing.contains(id));
        self.selection_scopes
            .mask_points
            .retain(|target| self.selection.contains(&target.item));
        self.selection_scopes
            .keyframes
            .retain(|target| self.selection.contains(&target.item));
    }
    fn split_at_playhead(&mut self) -> Result<()> {
        let at = self.playback.position()?;
        let mut under = Vec::new();
        let mut selected_under = Vec::new();
        for track in self.session.project().data().scene().timeline.iter() {
            for item in &track.items {
                if item.start < at && item.end().is_ok_and(|end| at < end) {
                    under.push(item.id);
                    if self.selection.contains(&item.id) {
                        selected_under.push(item.id);
                    }
                }
            }
        }
        // A nonempty selection owns the edit even when no selected item crosses the
        // playhead: splitting then is a no-op, never a fallback to unrelated clips.
        let targets = if self.selection.is_empty() {
            under
        } else {
            selected_under
        };
        if targets.is_empty() {
            return Ok(());
        }
        let mut created = Vec::new();
        self.session.transaction(|session| {
            for item in targets {
                created.extend(session.apply(Command::SplitAt { item, at })?.created);
            }
            Ok(())
        })?;
        self.selection = created
            .into_iter()
            .filter_map(|id| match id {
                CreatedId::Item(item) => Some(item),
                _ => None,
            })
            .collect();
        self.session.end_merge();
        Ok(())
    }
    fn delete_selected(&mut self, ripple: bool) -> Result<()> {
        if self.selection.is_empty() {
            return Ok(());
        }
        if ripple {
            let order = self.selection_by_end(true);
            self.session.transaction(|session| {
                for item in &order {
                    session.apply(Command::RippleDelete { item: *item })?;
                }
                Ok(())
            })?;
        } else {
            let items: Vec<ItemId> = self.selection.iter().copied().collect();
            self.session.apply(Command::Delete { items })?;
        }
        self.selection.clear();
        self.session.end_merge();
        Ok(())
    }
    fn selection_by_end(&self, descending: bool) -> Vec<ItemId> {
        let mut items: Vec<(Tick, ItemId)> = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter())
            .filter(|item| self.selection.contains(&item.id))
            .map(|item| (item.start, item.id))
            .collect();
        items.sort_by(|a, b| {
            if descending {
                b.0.cmp(&a.0)
            } else {
                a.0.cmp(&b.0)
            }
        });
        items.into_iter().map(|(_, id)| id).collect()
    }
    fn begin_drag(&mut self, item: ItemId, mode: DragMode, ui: &egui::Ui) {
        if self.drag.is_some() {
            self.finish_drag();
        }
        if !self.selection.contains(&item) {
            self.select_single(item);
        }
        let Some(press_x) = ui
            .ctx()
            .input(|input| input.pointer.press_origin().map(|origin| origin.x))
        else {
            return;
        };
        self.session.end_merge();
        if self.session.transaction_active()
            && let Err(error) = self.session.cancel_gesture()
        {
            self.error = Some(error.to_string());
            return;
        }
        if let Err(error) = self
            .session
            .set_selection(self.selection.clone())
            .and_then(|_| self.session.begin_gesture())
        {
            self.error = Some(error.to_string());
            return;
        }
        let mut origins = Vec::new();
        let mut source_range = None;
        for track in self.session.project().data().scene().timeline.iter() {
            for candidate in &track.items {
                let wanted = if mode == DragMode::Move {
                    self.selection.contains(&candidate.id)
                } else {
                    candidate.id == item
                };
                if !wanted {
                    continue;
                }
                if let Ok(end) = candidate.end() {
                    origins.push(DragOrigin {
                        item: candidate.id,
                        track: track.id,
                        start: candidate.start,
                        end,
                    });
                    if candidate.id == item {
                        source_range = Some(candidate.source_range);
                    }
                }
            }
        }
        let Some(source_range) = source_range else {
            let _ = self.session.cancel_gesture();
            return;
        };
        self.drag = Some(DragState {
            item,
            mode,
            press_x,
            origins,
            source_range,
            moved: false,
        });
    }
    /// Drive the active gesture from global pointer state. Release and update do not
    /// depend on the item or handle still being visible or hit-testable this frame.
    fn advance_drag(&mut self, ui: &egui::Ui, layout: &TimelineLayout, rows: &[TrackRow]) {
        if self.drag.is_none() {
            return;
        }
        let (pointer, hover, bypass, down) = ui.ctx().input(|input| {
            (
                input.pointer.interact_pos(),
                input.pointer.hover_pos(),
                input.modifiers.shift,
                input.pointer.primary_down(),
            )
        });
        if let Some(pointer) = pointer {
            let (_, max_scroll) = self.scroll_bounds(layout, self.session.project().duration());
            let step = if down {
                edge_auto_scroll(hover, layout.body, self.scroll, max_scroll)
            } else {
                0.0
            };
            if step != 0.0 {
                let before = self.scroll;
                self.scroll = (self.scroll + step / self.zoom).clamp(0.0, max_scroll);
                let step = (self.scroll - before) * self.zoom;
                if let Some(drag) = &mut self.drag
                    && drag.mode == DragMode::Move
                {
                    // Keep the grabbed content under the pointer while the
                    // viewport advances; trim already reads the scroll offset.
                    drag.press_x -= step;
                }
            }
            self.update_drag(pointer, layout.body.left(), bypass);
        }
        if !down {
            if let Some(pointer) = pointer
                && let Err(error) = self.drop_track(pointer, layout, rows, bypass)
            {
                self.error = Some(error.to_string());
                self.cancel_interaction();
                return;
            }
            self.finish_drag();
        }
    }
    fn drop_track(
        &mut self,
        pointer: egui::Pos2,
        layout: &TimelineLayout,
        rows: &[TrackRow],
        bypass: bool,
    ) -> Result<()> {
        let Some(drag) = self.drag.clone() else {
            return Ok(());
        };
        if drag.mode != DragMode::Move || drag.origins.len() != 1 || !layout.body.contains(pointer)
        {
            return Ok(());
        }
        let Some(origin) = drag.origins.first() else {
            return Ok(());
        };
        let mut destination = None;
        for row in rows {
            let top = layout.body.top() + row.top - self.vertical_scroll;
            let bottom = top + row.height;
            if pointer.y < top + 3.0 {
                destination = Some((
                    row.id,
                    Some(engine::TrackPlacement::Above { track: row.id }),
                ));
                break;
            }
            if pointer.y <= bottom - 3.0 {
                destination = Some((row.id, None));
                break;
            }
            destination = Some((
                row.id,
                Some(engine::TrackPlacement::Below { track: row.id }),
            ));
            if pointer.y <= bottom + TRACK_GAP / 2.0 {
                break;
            }
        }
        let Some((target, placement)) = destination else {
            return Ok(());
        };
        if placement.is_none() && target == origin.track {
            return Ok(());
        }
        let raw = Tick::from_seconds_f64(
            f64::from(pointer.x - drag.press_x) / f64::from(self.zoom),
            Rounding::Nearest,
        )?;
        let (delta, _) = self.snap_delta(raw, &drag.origins, bypass);
        let start = origin
            .start
            .checked_add(delta)
            .ok_or(engine::EditError::Invalid("time overflow"))?
            .max(Tick::ZERO);
        let target = if let Some(placement) = placement {
            let kind = self
                .session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .find(|t| t.id == origin.track)
                .ok_or(engine::EditError::NoTrack(origin.track))?
                .kind;
            let outcome = self.session.apply(Command::AddTrack {
                name: format!("{kind:?}").into(),
                kind,
                placement,
            })?;
            match outcome.created.first() {
                Some(CreatedId::Track(id)) => *id,
                _ => return Err(engine::EditError::Invalid("track allocation").into()),
            }
        } else {
            target
        };
        self.session.apply(Command::MoveItem {
            item: drag.item,
            track: target,
            start,
        })?;
        if let Some(drag) = &mut self.drag {
            drag.moved = true;
        }
        Ok(())
    }
    fn update_drag(&mut self, pointer: egui::Pos2, body_left: f32, bypass: bool) {
        let Some(mut drag) = self.drag.clone() else {
            return;
        };
        let result = match drag.mode {
            DragMode::Move => self.apply_drag_move(&drag, pointer.x, bypass),
            DragMode::TrimStart => {
                self.apply_drag_trim(&drag, pointer.x, body_left, bypass, DragMode::TrimStart)
            }
            DragMode::TrimEnd => {
                self.apply_drag_trim(&drag, pointer.x, body_left, bypass, DragMode::TrimEnd)
            }
        };
        let changed = match result {
            Ok(changed) => changed,
            Err(error) => {
                self.error = Some(error.to_string());
                false
            }
        };
        drag.moved |= changed;
        self.drag = Some(drag);
        if changed {
            self.sync_preview();
        }
    }
    fn apply_drag_move(&mut self, drag: &DragState, pointer_x: f32, bypass: bool) -> Result<bool> {
        let raw = Tick::from_seconds_f64(
            f64::from(pointer_x - drag.press_x) / f64::from(self.zoom),
            Rounding::Nearest,
        )?;
        let (snapped, indicator) = self.snap_delta(raw, &drag.origins, bypass);
        let (low, high) = self.move_bounds(&drag.origins);
        let delta = snapped.clamp(low, high);
        self.snap_indicator = indicator;
        // Always target the origin plus this delta. A return to the press point must
        // restore the gesture-entry positions rather than leave an earlier preview.
        let moves = drag
            .origins
            .iter()
            .map(|origin| engine::Move {
                item: origin.item,
                track: origin.track,
                start: origin.start + delta,
            })
            .collect();
        Ok(self.session.apply(Command::MoveItems { moves })?.changed)
    }
    fn apply_drag_trim(
        &mut self,
        drag: &DragState,
        pointer_x: f32,
        body_left: f32,
        bypass: bool,
        mode: DragMode,
    ) -> Result<bool> {
        let raw = Tick::from_seconds_f64(
            f64::from(self.scroll + (pointer_x - body_left) / self.zoom),
            Rounding::Nearest,
        )?;
        let exclude: BTreeSet<ItemId> = drag.origins.iter().map(|origin| origin.item).collect();
        let (snapped, indicator) = self.snap_tick(raw, &exclude, bypass);
        self.snap_indicator = indicator;
        let (low, high) = self.trim_bounds(drag, mode);
        let at = snapped.clamp(low, high);
        let command = match mode {
            DragMode::TrimStart => Command::TrimStart {
                item: drag.item,
                at,
            },
            DragMode::TrimEnd => Command::TrimEnd {
                item: drag.item,
                at,
            },
            DragMode::Move => return Ok(false),
        };
        Ok(self.session.apply(command)?.changed)
    }
    fn finish_drag(&mut self) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        let mut edited = false;
        if self.session.gesture_active() {
            let outcome = if drag.moved {
                edited = true;
                self.session.commit_gesture()
            } else {
                self.session.cancel_gesture()
            };
            if let Err(error) = outcome {
                self.error = Some(error.to_string());
            }
        }
        if edited {
            self.sync_preview();
        }
        self.snap_indicator = None;
        self.prune_selection();
    }
    fn cancel_interaction(&mut self) {
        if self.media_marquee.take().is_some() {
            self.media_selection.clear();
            self.media_anchor = None;
            self.session.end_merge();
            return;
        }
        if let Some(scrub) = self.scrub.take() {
            let changed = scrub.last != scrub.start;
            if self.session.gesture_active()
                && let Err(error) = self.session.cancel_gesture()
            {
                self.error = Some(error.to_string());
            }
            self.property_drafts.remove(&scrub.target.draft_key());
            self.property_focus = None;
            if changed {
                self.sync_preview();
            }
            return;
        }
        let track_drag = self.track_drag.take().is_some();
        if self.track_rename.take().is_some() || track_drag {
            self.track_rename_focus = false;
            return;
        }
        if self.effect_drag.take().is_some() {
            return;
        }
        if self.effect_reorder.take().is_some() {
            return;
        }
        if let Some(drag) = self.drag.take() {
            if self.session.gesture_active()
                && let Err(error) = self.session.cancel_gesture()
            {
                self.error = Some(error.to_string());
            }
            if drag.moved {
                self.sync_preview();
            }
            self.snap_indicator = None;
        } else if let Some((_, before, _)) = self.marquee.take() {
            self.selection = before;
            self.selection_changed();
        } else {
            self.selection_scopes.clear_specific(&mut self.selection);
            if let Err(e) = self.session.set_selection(self.selection.clone()) {
                self.error = Some(e.to_string());
            }
            if !self.media_selection.is_empty() {
                self.media_selection.clear();
                self.media_anchor = None;
                self.session.end_merge();
            }
        }
    }
    fn move_bounds(&self, origins: &[DragOrigin]) -> (Tick, Tick) {
        let tracks: BTreeSet<TrackId> = origins.iter().map(|origin| origin.track).collect();
        let selected: BTreeSet<ItemId> = origins.iter().map(|origin| origin.item).collect();
        let earliest = origins
            .iter()
            .map(|origin| origin.start)
            .min()
            .unwrap_or(Tick::ZERO);
        // One shared delta keeps the selected group rigid at the timeline zero edge.
        let mut low = Tick(earliest.0.saturating_neg());
        let mut high = Tick(i64::MAX / 4);
        for track in self.session.project().data().scene().timeline.iter() {
            if !tracks.contains(&track.id) {
                continue;
            }
            let track_origins: Vec<&DragOrigin> = origins
                .iter()
                .filter(|origin| origin.track == track.id)
                .collect();
            for item in &track.items {
                if selected.contains(&item.id) {
                    continue;
                }
                let Ok(end) = item.end() else {
                    continue;
                };
                let previous_end = track_origins
                    .iter()
                    .filter(|origin| origin.end <= item.start)
                    .map(|origin| origin.end)
                    .max();
                if let Some(previous_end) = previous_end {
                    high = high.min(item.start - previous_end);
                }
                let next_start = track_origins
                    .iter()
                    .filter(|origin| origin.start >= end)
                    .map(|origin| origin.start)
                    .min();
                if let Some(next_start) = next_start {
                    low = low.max(end - next_start);
                }
            }
        }
        (low, high)
    }
    fn trim_bounds(&self, drag: &DragState, mode: DragMode) -> (Tick, Tick) {
        let minimum = self.frame_ticks();
        let origin = drag
            .origins
            .iter()
            .find(|origin| origin.item == drag.item)
            .copied();
        let Some(origin) = origin else {
            return (Tick::ZERO, Tick::ZERO);
        };
        let mut low = match mode {
            DragMode::TrimStart => (origin.start - drag.source_range.start()).max(Tick::ZERO),
            DragMode::TrimEnd => origin.start + minimum,
            DragMode::Move => return (Tick::ZERO, Tick::ZERO),
        };
        let mut high = match mode {
            DragMode::TrimStart => origin.end - minimum,
            DragMode::TrimEnd => Tick(i64::MAX / 4),
            DragMode::Move => return (Tick::ZERO, Tick::ZERO),
        };
        for track in self.session.project().data().scene().timeline.iter() {
            if track.id != origin.track {
                continue;
            }
            for item in &track.items {
                if item.id == origin.item {
                    continue;
                }
                let Ok(end) = item.end() else {
                    continue;
                };
                match mode {
                    DragMode::TrimStart if end <= origin.start => low = low.max(end),
                    DragMode::TrimEnd if item.start >= origin.end => high = high.min(item.start),
                    _ => {}
                }
            }
        }
        if mode == DragMode::TrimEnd
            && let Some(asset) = self
                .session
                .project()
                .data()
                .assets
                .iter()
                .find(|asset| Some(asset.id()) == self.item_asset(drag.item))
            && let Some(duration) = asset.probe().duration
        {
            high = high.min(origin.start + (duration - drag.source_range.start()));
        }
        if high < low {
            high = low;
        }
        (low, high)
    }
    fn item_asset(&self, item: ItemId) -> Option<media::AssetId> {
        self.session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter())
            .find(|candidate| candidate.id == item)
            .and_then(|candidate| candidate.source_asset)
    }
    fn snap_points(&self, exclude: &BTreeSet<ItemId>) -> Vec<Tick> {
        let project = self.session.project();
        let data = project.data();
        let mut points = vec![Tick::ZERO, self.playback.position().unwrap_or(Tick::ZERO)];
        points.push(project.duration());
        points.extend(
            data.scene()
                .bookmarks
                .iter()
                .flat_map(|bookmark| [bookmark.start, bookmark.start + bookmark.duration]),
        );
        points.extend(
            data.scene()
                .timeline
                .iter()
                .flat_map(|track| track.items.iter())
                .filter(|item| !exclude.contains(&item.id))
                .flat_map(|item| [item.start, item.end().unwrap_or(item.start)]),
        );
        points
    }
    fn snap_delta(&self, raw: Tick, origins: &[DragOrigin], bypass: bool) -> (Tick, Option<Tick>) {
        if !self.snapping || bypass {
            return (raw, None);
        }
        let exclude: BTreeSet<ItemId> = origins.iter().map(|origin| origin.item).collect();
        let points = self.snap_points(&exclude);
        let threshold = self.snap_threshold();
        let mut best: Option<(Tick, Tick)> = None;
        for origin in origins {
            for edge in [origin.start, origin.end] {
                let current = edge + raw;
                for point in &points {
                    let difference = *point - current;
                    if difference.abs() <= threshold
                        && best.is_none_or(|(distance, _)| difference.abs() < distance.abs())
                    {
                        best = Some((difference, *point));
                    }
                }
            }
        }
        match best {
            Some((difference, point)) => (raw + difference, Some(point)),
            None => (raw, None),
        }
    }
    fn snap_tick(
        &self,
        raw: Tick,
        exclude: &BTreeSet<ItemId>,
        bypass: bool,
    ) -> (Tick, Option<Tick>) {
        if !self.snapping || bypass {
            return (raw, None);
        }
        let threshold = self.snap_threshold();
        let mut best: Option<(Tick, Tick)> = None;
        for point in self.snap_points(exclude) {
            let difference = point - raw;
            if difference.abs() <= threshold
                && best.is_none_or(|(distance, _)| difference.abs() < distance.abs())
            {
                best = Some((difference, point));
            }
        }
        match best {
            Some((difference, point)) => (raw + difference, Some(point)),
            None => (raw, None),
        }
    }
    fn snap_threshold(&self) -> Tick {
        Tick::from_seconds_f64(
            f64::from(SNAP_PIXELS) / f64::from(self.zoom),
            Rounding::Nearest,
        )
        .unwrap_or(Tick::ZERO)
    }
    fn tick_at(&self, x: f32, left: f32) -> Tick {
        Tick::from_seconds_f64(
            f64::from(self.scroll + (x - left) / self.zoom),
            Rounding::Nearest,
        )
        .unwrap_or(Tick::ZERO)
    }
    fn x_at(&self, at: Tick, left: f32) -> f32 {
        left + (at.seconds() as f32 - self.scroll) * self.zoom
    }
    pub(crate) fn theme(ctx: &egui::Context, dark: bool) {
        let mut visuals = if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        let text = theme::text(dark);
        let border = theme::border(dark);
        let field = theme::field(dark);
        let hover = theme::hover(dark);
        let separator = theme::separator(dark);
        let stroke = |color| egui::Stroke::new(1.0, color);
        visuals.panel_fill = theme::panel(dark);
        visuals.window_fill = theme::modal(dark);
        visuals.window_stroke = stroke(border);
        visuals.extreme_bg_color = field;
        visuals.faint_bg_color = separator;
        visuals.code_bg_color = field;
        visuals.warn_fg_color = theme::warning(dark);
        visuals.error_fg_color = theme::error_text(dark);
        visuals.hyperlink_color = theme::accent(dark);
        visuals.override_text_color = Some(text);
        visuals.selection.bg_fill = theme::accent(dark);
        visuals.selection.stroke = stroke(theme::on_accent(dark));
        visuals.text_cursor.stroke = stroke(theme::accent(dark));
        visuals.widgets.noninteractive.bg_fill = theme::panel(dark);
        visuals.widgets.noninteractive.weak_bg_fill = theme::panel(dark);
        visuals.widgets.noninteractive.bg_stroke = stroke(separator);
        visuals.widgets.noninteractive.fg_stroke = stroke(theme::secondary(dark));
        visuals.widgets.inactive.bg_fill = field;
        visuals.widgets.inactive.weak_bg_fill = field;
        visuals.widgets.inactive.bg_stroke = stroke(border);
        visuals.widgets.inactive.fg_stroke = stroke(text);
        visuals.widgets.hovered.bg_fill = hover;
        visuals.widgets.hovered.weak_bg_fill = hover;
        visuals.widgets.hovered.bg_stroke = stroke(border);
        visuals.widgets.hovered.fg_stroke = stroke(text);
        visuals.widgets.active.bg_fill = hover;
        visuals.widgets.active.weak_bg_fill = hover;
        visuals.widgets.active.bg_stroke = stroke(theme::accent(dark));
        visuals.widgets.active.fg_stroke = stroke(text);
        visuals.widgets.open.bg_fill = hover;
        visuals.widgets.open.weak_bg_fill = hover;
        visuals.widgets.open.bg_stroke = stroke(border);
        visuals.widgets.open.fg_stroke = stroke(text);
        ctx.set_visuals(visuals);
    }
    pub(crate) fn fonts(ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::empty();
        fonts.font_data.insert(
            "Inter".into(),
            std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
                "../../../assets/fonts/Inter-Regular.ttf"
            ))),
        );
        fonts.font_data.insert(
            "CJK".into(),
            Arc::new(egui::FontData::from_static(include_bytes!(
                "../../../assets/fonts/VibeCutSansCJK-Regular.otf"
            ))),
        );
        fonts.families.insert(
            egui::FontFamily::Proportional,
            vec!["Inter".into(), "CJK".into()],
        );
        fonts
            .families
            .insert(egui::FontFamily::Monospace, vec!["Inter".into()]);
        ctx.set_fonts(fonts);
    }
    /// Advance jobs and the device clock even when the OS suppresses painting.
    pub fn tick(&mut self) {
        if let Some(autosave) = self.session.autosave()
            && let Err(e) = autosave.tick()
        {
            self.persistence_error = Some(std::sync::Arc::new(e));
        }
        if let Err(e) = self.playback.poll() {
            self.error = Some(e.to_string());
        }
        if let Some(previews) = &mut self.effect_previews
            && let Err(e) = previews.tick()
        {
            self.error = Some(e.to_string());
        }
        if let Some(dialog) = &mut self.export
            && let Err(error) = dialog.tick(self.session.project(), self.preferences.export)
        {
            dialog.failed(error);
        }
        if let Some(subtitles) = &mut self.subtitles {
            subtitles.invalidate_options(&self.preferences.subtitles);
            let result = subtitles.tick(&mut self.session);
            let notice = subtitles.notice.take();
            match result {
                Ok(true) => self.sync_preview(),
                Ok(false) => {}
                Err(e) => self.error = Some(e.to_string()),
            }
            if let Some(count) = notice {
                let message = catalog::format(
                    self.preferences.locale,
                    Text::SubDone,
                    &[(
                        "count",
                        &catalog::number(self.preferences.locale, count as u64),
                    )],
                );
                self.set_toast(&message, None);
            }
        }
        self.apply_import_events();
        self.restore_asset_state();
        self.truncate_toast();
        if self
            .media_reveal
            .as_ref()
            .is_some_and(|reveal| Instant::now() >= reveal.until)
        {
            self.media_reveal = None;
        }
        if let Err(error) = self.remember_view() {
            self.error = Some(error.to_string());
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        // The gesture belongs to the item and field visible at pointer-down.
        // Agents can change selection or panel state between pointer events.
        if self
            .effect_reorder
            .as_ref()
            .is_some_and(|drag| self.selected_item_id() != Some(drag.item) || self.details)
        {
            self.effect_reorder = None;
        }
        if self
            .scrub
            .as_ref()
            .is_some_and(|scrub| self.selected_item_id() != Some(scrub.item) || self.details)
        {
            self.finish_scrub(false);
        }
        // History and external commands can remove the track being renamed.
        if self.track_rename.as_ref().is_some_and(|(id, _)| {
            !self
                .session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .any(|track| track.id == *id)
        }) {
            self.track_rename = None;
            self.track_rename_focus = false;
        }
        if !self.session.transaction_active()
            && let Err(e) = self.session.set_selection(self.selection.clone())
        {
            self.error = Some(e.to_string());
        }
        if !self.configured {
            Self::fonts(ui.ctx());
            icons::install(ui.ctx());
            self.configured = true;
        }
        let (dropped, hovering) = ui.ctx().input(|input| {
            (
                input
                    .raw
                    .dropped_files
                    .iter()
                    .map(|file| file.path().to_owned())
                    .collect::<Vec<_>>(),
                !input.raw.hovered_files.is_empty(),
            )
        });
        if !dropped.is_empty()
            && let Err(error) = self.drop_files(dropped)
        {
            self.import_notice = Some(error_detail(&error));
        }
        Self::theme(ui.ctx(), self.preferences.dark);
        ui.style_mut().interaction.selectable_labels = false;
        self.widgets.clear();
        self.panels.clear();
        self.tick();
        // Undo and commands can remove a scene without the sheet's delete action.
        if self
            .scene_rename
            .as_ref()
            .is_some_and(|(scene, _)| self.session.project().data().scenes.get(*scene).is_none())
        {
            self.scene_rename = None;
        }
        if !self.export.as_ref().is_some_and(|d| d.open)
            && !self.subtitles.as_ref().is_some_and(|s| s.modal_open())
        {
            self.keyboard(ui.ctx());
        }
        let rect = ui.max_rect();
        let rect = Rect::from_min_size(rect.min, rect.size().max(Vec2::ZERO));
        ui.painter()
            .rect_filled(rect, 0.0, theme::window(self.preferences.dark));
        let top = Rect::from_min_size(rect.min, Vec2::new(rect.width(), rect.height().min(52.0)));
        let body = Rect::from_min_max(egui::pos2(rect.left(), top.bottom()), rect.max);
        let timeline_height = body.height() * self.preferences.panels[2];
        let upper = Rect::from_min_max(
            body.min,
            egui::pos2(body.right(), body.bottom() - timeline_height),
        );
        let timeline = Rect::from_min_max(egui::pos2(body.left(), upper.bottom()), body.max);
        let left_w = upper.width() * self.preferences.panels[0];
        let right_w = upper.width() * self.preferences.panels[1];
        let left = Rect::from_min_size(upper.min, Vec2::new(left_w, upper.height()));
        let right = Rect::from_min_max(egui::pos2(upper.right() - right_w, upper.top()), upper.max);
        let preview = Rect::from_min_max(
            egui::pos2(left.right(), upper.top()),
            egui::pos2(right.left(), upper.bottom()),
        );
        for (id, area) in [
            ("header", top),
            ("media", left),
            ("preview", preview),
            ("inspector", right),
            ("timeline", timeline),
        ] {
            self.panels.insert(id, area);
            let fill = if id == "header" {
                theme::window(self.preferences.dark)
            } else {
                theme::panel(self.preferences.dark)
            };
            ui.painter().rect_filled(area, 0.0, fill);
            let mut child = ui.new_child(egui::UiBuilder::new().id_salt(id).max_rect(
                area.shrink2(Vec2::new(
                    12.0_f32.min(area.width() / 2.0),
                    12.0_f32.min(area.height() / 2.0),
                )),
            ));
            child.set_clip_rect(area);
            if area.width() <= 0.0 || area.height() <= 0.0 {
                continue;
            }
            match id {
                "header" => self.header(&mut child),
                "media" => self.assets(&mut child),
                "preview" => self.preview(&mut child),
                "inspector" => self.inspector(&mut child),
                _ => self.timeline(&mut child),
            }
        }
        let separator = theme::separator(self.preferences.dark);
        for (id, area, axis) in [
            (
                "layout.media",
                Rect::from_center_size(
                    egui::pos2(left.right(), upper.center().y),
                    Vec2::new(8.0, upper.height()),
                ),
                0,
            ),
            (
                "layout.inspector",
                Rect::from_center_size(
                    egui::pos2(right.left(), upper.center().y),
                    Vec2::new(8.0, upper.height()),
                ),
                1,
            ),
            (
                "layout.timeline",
                Rect::from_center_size(
                    egui::pos2(timeline.center().x, timeline.top()),
                    Vec2::new(timeline.width(), 8.0),
                ),
                2,
            ),
        ] {
            let response = ui.interact(area, Id::unique(id), egui::Sense::drag());
            self.remember(id, &response);
            if response.drag_started() {
                self.layout_drag = Some(self.preferences.panels);
            }
            if response.dragged() {
                let delta = response.drag_delta();
                let fraction = match axis {
                    0 | 1 => delta.x / upper.width(),
                    _ => delta.y / body.height(),
                };
                self.preferences.panels = drag_panel(self.preferences.panels, axis, fraction);
            }
            if response.drag_stopped()
                && let Some(before) = self.layout_drag.take()
            {
                self.record_layout_change(before);
            }
        }
        for (a, b) in [
            (top.left_bottom(), top.right_bottom()),
            (left.right_top(), left.right_bottom()),
            (right.left_top(), right.left_bottom()),
            (timeline.left_top(), timeline.right_top()),
        ] {
            ui.painter()
                .line_segment([a, b], egui::Stroke::new(1.0, separator));
        }
        self.scenes_sheet(ui.ctx());
        self.media_delete_dialog(ui.ctx());
        self.import_overlay(ui.ctx());
        self.drop_overlay(ui.ctx(), hovering);
        self.toast(ui.ctx());
        self.export_dialog(ui.ctx());
        if let Some(subtitles) = &mut self.subtitles {
            let widgets = subtitles.dialogs(
                ui.ctx(),
                &self.session,
                &self.preferences.subtitles,
                self.preferences.locale,
            );
            for (id, response) in widgets {
                self.remember(id, &response);
            }
        }
        self.model_dialog(ui.ctx());
        if self
            .scrub
            .as_ref()
            .is_some_and(|scrub| !self.widgets.contains_key(&scrub.target.widget_id()))
        {
            self.finish_scrub(false);
        }
        if self.effect_reorder.as_ref().is_some_and(|drag| {
            !self
                .widgets
                .contains_key(&format!("effects.row.{}.handle", drag.effect))
        }) {
            self.effect_reorder = None;
        }
        self.finish_effect_drag(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(
                if self.playback.playing() { 8 } else { 33 },
            ));
    }
    fn keyboard(&mut self, ctx: &egui::Context) {
        if self.media_delete_confirm.is_some() {
            if consume(ctx, egui::Modifiers::NONE, egui::Key::Escape) {
                self.media_delete_confirm = None;
            }
            return;
        }
        if self.scenes_open {
            if (self.scene_confirm || self.scene_rename.is_none())
                && consume(ctx, egui::Modifiers::NONE, egui::Key::Escape)
            {
                if self.scene_confirm {
                    self.scene_confirm = false;
                } else {
                    self.scenes_open = false;
                }
            }
            return;
        }
        if self.marquee.is_some() || self.media_marquee.is_some() {
            if consume(ctx, egui::Modifiers::NONE, egui::Key::Escape) {
                self.cancel_interaction();
            }
            return;
        }
        if self.track_rename.is_some() && consume(ctx, egui::Modifiers::NONE, egui::Key::Escape) {
            self.track_rename = None;
            self.track_rename_focus = false;
            return;
        }
        if self.subtitles.as_ref().is_some_and(|s| s.editing(ctx))
            || self.rename.is_some()
            || self.property_focus.is_some()
            || self.track_rename.is_some()
            || ctx.memory(|m| m.focused()).is_some_and(|id| {
                [
                    "inspector.audio_gain",
                    "inspector.opacity",
                    "inspector.text_content",
                    "inspector.text_font",
                    "inspector.text_size",
                    "inspector.text_color",
                    "inspector.text_background_color",
                    "inspector.text_position_x",
                    "inspector.text_position_y",
                ]
                .iter()
                .any(|field| id == Id::unique(field))
            })
        {
            return;
        }
        // Most specific chords first: consume_key ignores extra Shift/Alt modifiers.
        let command_shift = egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT);
        if consume(ctx, command_shift, egui::Key::Z)
            || consume(ctx, egui::Modifiers::COMMAND, egui::Key::Y)
        {
            self.action("edit.redo");
        }
        if consume(ctx, egui::Modifiers::COMMAND, egui::Key::Z) {
            self.action("edit.undo");
        }
        if consume(ctx, egui::Modifiers::COMMAND, egui::Key::B) {
            self.action("timeline.split");
        }
        if consume(ctx, egui::Modifiers::COMMAND, egui::Key::A) {
            self.action("timeline.select_all");
        }
        if consume(ctx, egui::Modifiers::SHIFT, egui::Key::Delete)
            || consume(ctx, egui::Modifiers::SHIFT, egui::Key::Backspace)
        {
            self.action("timeline.ripple_delete");
        } else if consume(ctx, egui::Modifiers::NONE, egui::Key::Delete)
            || consume(ctx, egui::Modifiers::NONE, egui::Key::Backspace)
        {
            self.action("timeline.delete");
        }
        if consume(ctx, egui::Modifiers::NONE, egui::Key::N) {
            self.action("timeline.snap");
        }
        if consume(ctx, egui::Modifiers::NONE, egui::Key::Escape) {
            self.cancel_interaction();
        }
        if consume(ctx, egui::Modifiers::SHIFT, egui::Key::ArrowLeft) {
            self.action("transport.back");
        }
        if consume(ctx, egui::Modifiers::SHIFT, egui::Key::ArrowRight) {
            self.action("transport.forward");
        }
        if consume(ctx, egui::Modifiers::NONE, egui::Key::ArrowLeft) {
            self.action("transport.frame_back");
        }
        if consume(ctx, egui::Modifiers::NONE, egui::Key::ArrowRight) {
            self.action("transport.frame_forward");
        }
        for (key, id) in [
            (egui::Key::Space, "transport.play"),
            (egui::Key::K, "transport.pause"),
            (egui::Key::J, "transport.back"),
            (egui::Key::L, "transport.forward"),
            (egui::Key::Home, "transport.start"),
            (egui::Key::End, "transport.end"),
        ] {
            if consume(ctx, egui::Modifiers::NONE, key) {
                self.action(id);
            }
        }
    }
    /// Accept the native export save picker's result without opening OS UI in tests.
    pub fn export_picked_file(&mut self, path: PathBuf) -> Result<()> {
        let protected = self
            .session
            .autosave()
            .map(|s| s.protected_export_paths())
            .transpose()?
            .unwrap_or_default();
        let dialog = self
            .export
            .as_mut()
            .ok_or(Error::Invalid("export dialog closed"))?;
        if let Err(error) = dialog.save_to(path, protected) {
            dialog.failed(error);
        }
        Ok(())
    }

    /// Cancelling the native save dialog keeps the finished job for a later save.
    pub fn export_save_cancelled(&mut self) {
        if let Some(dialog) = &mut self.export {
            dialog.save_cancelled();
        }
    }
    /// Default filename and last directory for the native save dialog.
    pub fn export_save_defaults(&self) -> (String, Option<PathBuf>) {
        (
            crate::export::default_file_name(
                self.session.project().data().name.as_str(),
                self.export_save_format().extension(),
            ),
            self.preferences.export_directory.clone(),
        )
    }
    /// The completed attempt owns its container even if settings changed after failure.
    pub fn export_save_format(&self) -> crate::export::ExportFormat {
        self.export
            .as_ref()
            .and_then(|dialog| dialog.inspect().attempt)
            .map_or(self.preferences.export.format, |options| options.format)
    }
    fn export_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.export else {
            return;
        };
        let mut options = self.preferences.export;
        let output = dialog.ui(
            ctx,
            self.preferences.locale,
            self.preferences.dark,
            &mut options,
            self.session.project(),
            self.texture.as_ref(),
        );
        match output.action {
            Some(crate::export::Action::Start) => {
                let result = if self.session.transaction_active() {
                    Err(Error::Engine(engine::EditError::TransactionActive))
                } else {
                    dialog.start(self.session.project().clone(), options)
                };
                if let Err(error) = result {
                    dialog.failed(error);
                }
            }
            Some(crate::export::Action::Retry) => {
                let result = if self.session.transaction_active() {
                    Err(Error::Engine(engine::EditError::TransactionActive))
                } else {
                    dialog.retry(self.session.project().clone())
                };
                if let Err(error) = result {
                    dialog.failed(error);
                }
            }
            Some(crate::export::Action::Save) => {
                self.lifecycle_action = Some(crate::lifecycle::EditorAction::ExportSave);
            }
            None => {}
        }
        if let Some(request) =
            dialog.take_save_request(self.session.project(), self.preferences.export)
        {
            let _ = request;
            self.lifecycle_action = Some(crate::lifecycle::EditorAction::ExportSave);
        }
        if let Some(saved) = dialog.take_saved() {
            self.preferences.export_directory = saved.parent().map(Path::to_owned);
            self.set_toast(
                self.tr(Text::ExportSaved),
                Some(saved.to_string_lossy().into_owned()),
            );
        }
        if options != self.preferences.export
            && let Err(error) = self.dispatch(
                PREFERENCE_COMMAND,
                serde_json::json!({"key": "export", "value": options}),
            )
        {
            self.error = Some(error.to_string());
        }
        for (id, response) in output.widgets {
            self.remember(id, &response);
        }
    }
    /// Instantiate the one shared model cache on first use; it never fetches then.
    pub fn open_models(&mut self) -> Result<()> {
        if self.models.is_none() {
            self.models = Some(crate::model_manager::ModelManager::open_default(
                self.playback.budget().clone(),
            )?);
        }
        if let Some(models) = &mut self.models {
            models.show();
        }
        Ok(())
    }
    /// Open the manager on a specific missing model, as `/09` does.
    pub fn open_model(&mut self, id: &str) -> Result<()> {
        if self.models.is_none() {
            self.models = Some(crate::model_manager::ModelManager::open_default(
                self.playback.budget().clone(),
            )?);
        }
        self.models
            .as_mut()
            .ok_or(Error::Invalid("model manager unavailable"))?
            .open_model(id)
    }
    /// Inject cache/transport/IO ownership for composition and tests.
    pub fn set_model_manager(&mut self, models: crate::model_manager::ModelManager) {
        self.models = Some(models);
    }
    pub fn models(&self) -> Option<&crate::model_manager::ModelManager> {
        self.models.as_ref()
    }
    /// Initialize the panel without any download or provider construction.
    pub fn prepare_subtitles(&mut self) -> Result<()> {
        if self.subtitles.is_none() {
            self.subtitles = Some(crate::subtitles::Subtitles::new(
                self.playback.budget().clone(),
            )?);
        }
        Ok(())
    }
    fn captions_panel(&mut self, ui: &mut egui::Ui) {
        if let Err(e) = self.prepare_subtitles() {
            self.error = Some(e.to_string());
            return;
        }
        let before = self.session.project().clone();
        let mut options = self.preferences.subtitles.clone();
        let at = self.playback.position().unwrap_or(Tick::ZERO);
        if let Some(subtitles) = &mut self.subtitles {
            let widgets = subtitles.ui(
                ui,
                &mut self.session,
                &mut options,
                &self.selection,
                at,
                self.preferences.locale,
                self.preferences.dark,
            );
            let seek = subtitles.requested_seek.take();
            let model = subtitles.requested_model.take();
            for (id, response) in widgets {
                self.remember(id, &response);
            }
            if let Some(at) = seek
                && let Err(e) = self.seek(at)
            {
                self.error = Some(e.to_string());
            }
            if let Some(model) = model
                && let Err(e) = self.open_model(&model)
            {
                self.error = Some(e.to_string());
            }
        }
        if options != self.preferences.subtitles {
            match serde_json::to_value(options)
                .map_err(Error::from)
                .and_then(|value| self.set_preference("subtitles", value))
            {
                Ok(()) => {}
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        if !Arc::ptr_eq(&before, self.session.project()) {
            self.sync_preview();
        }
    }
    fn model_dialog(&mut self, ctx: &egui::Context) {
        let Some(models) = &mut self.models else {
            return;
        };
        let output = models.ui(ctx, self.preferences.locale, self.preferences.dark);
        for (id, response) in output.widgets {
            self.remember(id, &response);
        }
    }
    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.label(
                egui::RichText::new(self.tr(Text::AppName))
                    .strong()
                    .size(18.0),
            );
            ui.separator();
            if self.button(ui, "project.back", Icon::ArrowLeft, Text::Projects, true) {
                self.action("project.back");
            }
            if self.rename.is_some() {
                let mut name = self.rename.take().unwrap_or_default();
                let response = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .id_salt("project.rename.input")
                        .desired_width(230.0),
                );
                self.remember("project.rename.input", &response);
                if self.rename_focus {
                    response.request_focus();
                    self.rename_focus = false;
                }
                let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                let commit = ui.input(|i| i.key_pressed(egui::Key::Enter));
                if commit && !cancel {
                    if let Err(e) = self
                        .session
                        .execute("project.rename", serde_json::json!({"name": name}))
                    {
                        self.error = Some(e.to_string());
                        self.rename = Some(name);
                    }
                } else if !cancel {
                    self.rename = Some(name);
                }
            } else {
                let image = icons::tinted(Icon::PencilSimple, BUTTON_ICON, self.text_color());
                let response = ui.add_sized(
                    [220.0, 26.0],
                    egui::Button::image_and_text(
                        image,
                        egui::RichText::new(self.session.project().data().name.as_str()),
                    )
                    .truncate(),
                );
                self.remember_with_icon(
                    "project.rename",
                    Some(Icon::PencilSimple.name()),
                    &response,
                );
                if response.clicked() {
                    self.action("project.rename");
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.button(ui, "file.export", Icon::Export, Text::Export, true) {
                    self.action("file.export");
                }
                if self.button(
                    ui,
                    "project.save_as",
                    Icon::FloppyDiskBack,
                    Text::SaveAs,
                    true,
                ) {
                    self.action("project.save_as");
                }
                if self.button(ui, "project.save", Icon::FloppyDisk, Text::Save, true) {
                    self.action("project.save");
                }
                if self.button(ui, "appearance.theme", Icon::CircleHalf, Text::Theme, true) {
                    self.action("appearance.theme");
                }
                if self.button(
                    ui,
                    "appearance.locale",
                    Icon::Translate,
                    Text::Language,
                    true,
                ) {
                    self.action("appearance.locale");
                }
                if self.button(ui, "layout.reset", Icon::CornersOut, Text::Reset, true) {
                    self.action("layout.reset");
                }
                if self.button(ui, "models.open", Icon::File, Text::Models, true) {
                    self.action("models.open");
                }
            });
        });
    }
    /// The assets panel: a scrollable vertical tab bar and the selected view.
    fn assets(&mut self, ui: &mut egui::Ui) {
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let bar = egui::vec2(36.0, height);
            let mut clicked = None;
            let mut shown = None;
            ui.allocate_ui_with_layout(bar, egui::Layout::top_down(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                let output = egui::ScrollArea::vertical()
                    .id_salt("assets.tabs")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for tab in AssetsTab::ALL {
                            let id = format!("assets.tab.{}", tab.id());
                            let active = self.preferences.assets_tab == tab;
                            let label = self.tr(tab.label());
                            if self.icon_only(ui, &id, tab.icon(), label, None, Some(active), true)
                            {
                                clicked = Some(tab);
                            }
                        }
                    });
                shown = Some(output);
            });
            if let Some(tab) = clicked
                && let Err(error) = self.remember_assets_tab(tab)
            {
                self.error = Some(error.to_string());
            }
            if let Some(output) = shown {
                let dark = self.preferences.dark;
                scroll_fades(ui, &output, dark);
            }
            let separator = theme::separator(self.preferences.dark);
            ui.painter().line_segment(
                [
                    egui::pos2(ui.cursor().min.x, ui.cursor().min.y),
                    egui::pos2(ui.cursor().min.x, ui.cursor().min.y + height),
                ],
                egui::Stroke::new(1.0, separator),
            );
            ui.add_space(12.0);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| match self.preferences.assets_tab {
                    AssetsTab::Media => self.media_view(ui),
                    AssetsTab::Effects => self.effects_view(ui),
                    AssetsTab::Captions => self.captions_panel(ui),
                    tab => self.assets_stub(ui, tab),
                },
            );
        });
    }
    fn effects_view(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new(self.tr(Text::Effects))
                .size(16.0)
                .strong(),
        );
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(self.tr(Text::EffectDropHint))
                .small()
                .color(self.secondary()),
        );
        ui.add_space(8.0);
        let width = ((ui.available_width() - 8.0) / 2.0).clamp(112.0, 148.0);
        egui::ScrollArea::vertical()
            .id_salt("effects.tiles")
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for definition in project::effect_definitions() {
                        self.effect_tile(ui, definition, width);
                    }
                });
            });
    }
    fn effect_tile(
        &mut self,
        ui: &mut egui::Ui,
        definition: &project::EffectDefinition,
        width: f32,
    ) {
        let kind = definition.kind;
        let params = effect_preview_params(definition);
        if let (Some(previews), Some(params)) = (&mut self.effect_previews, params.as_ref()) {
            previews.ensure(kind, params);
        }
        let status = self
            .effect_previews
            .as_ref()
            .and_then(|previews| previews.status(kind));
        if !matches!(
            status,
            Some(crate::effect_previews::EffectPreviewStatus::Ready)
        ) {
            self.effect_textures.remove(kind);
        } else if !self.effect_textures.contains_key(kind)
            && let Some(image) = self
                .effect_previews
                .as_ref()
                .and_then(|previews| previews.image(kind))
        {
            let texture = crate::effect_previews::preview_texture(ui.ctx(), kind, image);
            self.effect_textures.insert(kind.to_owned(), texture);
        }
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, 118.0), egui::Sense::click_and_drag());
        self.remember(format!("effects.tile.{kind}"), &response);
        let state = match status {
            Some(crate::effect_previews::EffectPreviewStatus::Pending) => "pending",
            Some(crate::effect_previews::EffectPreviewStatus::Ready) => "ready",
            Some(crate::effect_previews::EffectPreviewStatus::Failed) => "failed",
            None => "failed",
        };
        self.remember(format!("effects.tile.{kind}.{state}"), &response);
        if response.drag_started() {
            self.effect_drag = Some(kind.to_owned());
        }
        ui.painter().rect_filled(rect, 4.0, self.control_fill());
        let preview = Rect::from_min_max(
            egui::pos2(rect.left() + 6.0, rect.top() + 6.0),
            egui::pos2(rect.right() - 6.0, rect.bottom() - 26.0),
        );
        match status {
            Some(crate::effect_previews::EffectPreviewStatus::Ready) => {
                if let Some(texture) = self.effect_textures.get(kind) {
                    ui.painter().image(
                        texture.id(),
                        preview,
                        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
            }
            Some(crate::effect_previews::EffectPreviewStatus::Pending) => {
                ui.painter().text(
                    preview.center(),
                    Align2::CENTER_CENTER,
                    self.tr(Text::EffectPreviewPending),
                    FontId::proportional(11.0),
                    self.secondary(),
                );
            }
            Some(crate::effect_previews::EffectPreviewStatus::Failed) | None => {
                ui.painter().text(
                    preview.center(),
                    Align2::CENTER_CENTER,
                    self.tr(Text::EffectPreviewFailed),
                    FontId::proportional(11.0),
                    self.secondary(),
                );
                let retry = Rect::from_center_size(
                    egui::pos2(preview.right() - 16.0, preview.bottom() - 16.0),
                    Vec2::splat(24.0),
                );
                let response = ui.interact(
                    retry,
                    Id::unique(("effects.preview.retry", kind)),
                    egui::Sense::click(),
                );
                let label = self.tr(Text::EffectPreviewRetry);
                icons::tinted(Icon::ArrowClockwise, BUTTON_ICON, self.secondary())
                    .paint_at(ui, retry);
                let response = response.on_hover_text(label);
                self.remember_with_icon(
                    format!("effects.preview.retry.{kind}"),
                    Some(Icon::ArrowClockwise.name()),
                    &response,
                );
                if response.clicked() {
                    if let (Some(previews), Some(params)) =
                        (&mut self.effect_previews, params.as_ref())
                    {
                        previews.retry(kind, params);
                    }
                    self.effect_textures.remove(kind);
                }
            }
        }
        let plus = Rect::from_center_size(
            egui::pos2(rect.right() - 15.0, rect.top() + 15.0),
            Vec2::splat(24.0),
        );
        let response = ui.interact(
            plus,
            Id::unique(("effects.add", kind)),
            egui::Sense::click(),
        );
        let label = self.tr(Text::EffectAdd);
        ui.painter().circle_filled(
            plus.center(),
            9.0,
            self.accent()
                .gamma_multiply(if response.hovered() { 1.0 } else { 0.8 }),
        );
        icons::tinted(Icon::Plus, 12.0, theme::on_accent(self.preferences.dark))
            .paint_at(ui, plus.shrink(6.0));
        let response = response.on_hover_text(label);
        self.remember_with_icon(
            format!("effects.add.{kind}"),
            Some(Icon::Plus.name()),
            &response,
        );
        if response.clicked() {
            self.add_effect_from_selection(kind);
        }
        ui.painter().text(
            egui::pos2(rect.left() + 8.0, rect.bottom() - 14.0),
            Align2::LEFT_CENTER,
            self.effect_name(definition),
            FontId::proportional(12.0),
            self.text_color(),
        );
    }
    fn add_effect_from_selection(&mut self, kind: &str) {
        match self.selected_item_id() {
            Some(item) => self.add_effect_to_item(item, kind),
            None => {
                let text = self.tr(Text::EffectSelectClip).to_owned();
                self.set_toast(&text, None);
            }
        }
    }
    fn effect_name(&self, definition: &project::EffectDefinition) -> &'static str {
        match definition.kind {
            "blur" => self.tr(Text::EffectBlur),
            _ => definition.name,
        }
    }
    fn finish_effect_drag(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.effect_drag.clone() else {
            return;
        };
        if !ctx.input(|input| input.pointer.any_released()) {
            return;
        }
        self.effect_drag = None;
        let position = ctx.input(|input| {
            input
                .pointer
                .interact_pos()
                .or_else(|| input.pointer.latest_pos())
        });
        let target = position.and_then(|position| {
            self.effect_drop_targets
                .iter()
                .find(|(_, rect)| rect.contains(position))
                .map(|(item, _)| *item)
        });
        if let Some(item) = target {
            self.add_effect_to_item(item, &kind);
        } else {
            let text = self.tr(Text::EffectDropMiss).to_owned();
            self.set_toast(&text, None);
        }
    }
    fn assets_stub(&mut self, ui: &mut egui::Ui, tab: AssetsTab) {
        ui.label(
            egui::RichText::new(self.tr(tab.label()))
                .size(16.0)
                .strong(),
        );
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(self.tr(Text::PanelStub))
                .small()
                .color(self.secondary()),
        );
    }
    fn remember_assets_tab(&mut self, tab: AssetsTab) -> Result<()> {
        if self.preferences.assets_tab != tab {
            let flushed = if let Some(panel) = &mut self.subtitles {
                panel.flush_drafts(&mut self.session)?
            } else {
                false
            };
            if flushed {
                self.sync_preview();
            }
            self.preferences
                .set("assets_tab", serde_json::to_value(tab)?)?;
        }
        Ok(())
    }
    fn media_view(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new(self.tr(Text::Media))
                .size(16.0)
                .strong(),
        );
        ui.add_space(8.0);
        self.refresh_media_sizes();
        let importing = self.importing();
        self.media_toolbar(ui, importing);
        if self.button(ui, "text.insert", Icon::TextT, Text::TextItem, true) {
            self.action("text.insert");
        }
        // Reserve the line whether or not a selection exists so selecting an
        // asset never shifts the grid under a subsequent click.
        let count = if !importing && !self.media_selection.is_empty() {
            catalog::format_count(
                self.preferences.locale,
                self.media_selection.len() as u64,
                Text::SelectedCountOne,
                Text::SelectedCountOther,
            )
        } else {
            String::new()
        };
        ui.add_sized(
            [ui.available_width(), 20.0],
            egui::Label::new(
                egui::RichText::new(count)
                    .size(14.0)
                    .color(self.secondary()),
            ),
        );
        ui.add_space(8.0);
        let locale = self.preferences.locale;
        let order = self.media_order();
        let sizes = self.media_size_map();
        let by_id: BTreeMap<AssetId, &SourceAsset> = self
            .session
            .project()
            .data()
            .assets
            .iter()
            .map(|asset| (asset.id(), asset))
            .collect();
        let mut rows: Vec<AssetRow> = order
            .iter()
            .filter_map(|id| {
                by_id.get(id).map(|asset| {
                    let state = self
                        .asset_states
                        .get(id)
                        .copied()
                        .unwrap_or(AssetState::Probing);
                    asset_row(asset, locale, sizes.get(id).copied().flatten(), state)
                })
            })
            .collect();
        rows.extend(self.pending_imports.iter().map(|(id, path)| AssetRow {
            id: *id,
            name: file_name(path),
            info: asset_state_text(locale, AssetState::Probing),
            brief: asset_state_text(locale, AssetState::Probing),
            icon: Icon::UploadSimple,
            state: AssetState::Probing,
            duration: None,
        }));
        if rows.is_empty() {
            ui.label(self.tr(Text::EmptyMedia));
        } else {
            match self.preferences.media_view {
                MediaView::Grid => self.media_grid(ui, &rows, importing),
                MediaView::List => self.media_list(ui, &rows, importing),
            }
            if self.media_selection.is_empty() && !importing {
                ui.label(
                    egui::RichText::new(self.tr(Text::SelectAssets))
                        .small()
                        .color(self.secondary()),
                );
            }
        }
        if !self.import_failures.is_empty() {
            ui.separator();
            ui.label(
                egui::RichText::new(self.tr(Text::ImportIssues))
                    .size(14.0)
                    .strong(),
            );
            for failure in &self.import_failures {
                let name = file_name(&failure.path);
                ui.label(
                    egui::RichText::new(catalog::format(
                        self.preferences.locale,
                        Text::FailureDetail,
                        &[("name", &name), ("reason", &failure.reason)],
                    ))
                    .small()
                    .color(self.secondary()),
                );
            }
        }
        if let Some(notice) = &self.import_notice {
            ui.separator();
            ui.label(egui::RichText::new(notice).small().color(self.secondary()));
        }
    }
    fn media_toolbar(&mut self, ui: &mut egui::Ui, importing: bool) {
        ui.horizontal(|ui| {
            let (icon, label) = match self.preferences.media_view {
                MediaView::Grid => (Icon::ListBullets, Text::SwitchToList),
                MediaView::List => (Icon::SquaresFour, Text::SwitchToGrid),
            };
            if self.icon_only(
                ui,
                "media.view",
                icon,
                self.tr(label),
                None,
                None,
                !importing,
            ) {
                self.action("media.view");
            }
            ui.add_enabled_ui(!importing, |ui| self.media_sort_menu(ui));
            if self.button(
                ui,
                "file.import",
                Icon::UploadSimple,
                Text::Import,
                !importing,
            ) {
                self.action("file.import");
            }
            if self.icon_only(
                ui,
                "media.insert",
                Icon::Plus,
                self.tr(Text::AddToTimeline),
                None,
                None,
                !importing && !self.media_selection.is_empty(),
            ) {
                self.action("media.insert");
            }
        });
    }
    fn media_sort_label(&self, key: MediaSortKey) -> &'static str {
        self.tr(match key {
            MediaSortKey::Name => Text::SortName,
            MediaSortKey::Type => Text::SortType,
            MediaSortKey::Duration => Text::SortDuration,
            MediaSortKey::Size => Text::SortSize,
        })
    }
    fn media_sort_menu(&mut self, ui: &mut egui::Ui) {
        let current = self.preferences.media_sort;
        let arrow = match current.order {
            MediaSortOrder::Asc => "↑",
            MediaSortOrder::Desc => "↓",
        };
        let selected = format!("{} {arrow}", self.media_sort_label(current.key));
        let combo = egui::ComboBox::from_id_salt(Id::unique("media.sort"))
            .selected_text(selected)
            .width(120.0)
            .show_ui(ui, |ui| {
                for key in MediaSortKey::ALL {
                    let active = current.key == key;
                    let arrow = if active { arrow } else { "" };
                    let response = ui.selectable_label(
                        active,
                        format!("{} {arrow}", self.media_sort_label(key)),
                    );
                    self.remember(format!("media.sort.{}", key.id()), &response);
                    if response.clicked() {
                        self.action(&format!("media.sort.{}", key.id()));
                        ui.close();
                    }
                }
            });
        let order = self.tr(match current.order {
            MediaSortOrder::Asc => Text::SortAscending,
            MediaSortOrder::Desc => Text::SortDescending,
        });
        let tooltip = catalog::format(
            self.preferences.locale,
            Text::SortTooltip,
            &[
                ("key", self.media_sort_label(current.key)),
                ("order", order),
            ],
        );
        let response = combo.response.on_hover_text(tooltip.clone());
        self.remember("media.sort", &response);
        if let Some(state) = self.widgets.get_mut("media.sort") {
            state.tooltip = Some(tooltip);
        }
    }
    fn media_viewport_height(&self, ui: &egui::Ui) -> f32 {
        let reserved = if self.import_failures.is_empty() && self.import_notice.is_none() {
            16.0
        } else {
            120.0
        };
        (ui.clip_rect().bottom() - ui.cursor().min.y - reserved).max(80.0)
    }
    fn media_grid(&mut self, ui: &mut egui::Ui, rows: &[AssetRow], importing: bool) {
        let width = (ui.clip_rect().right() - ui.cursor().min.x).max(0.0);
        let columns = ((width + MEDIA_CARD_GAP) / (MEDIA_CARD_WIDTH + MEDIA_CARD_GAP))
            .floor()
            .max(1.0) as usize;
        let content_height = rows.len().div_ceil(columns) as f32
            * (MEDIA_CARD_HEIGHT + MEDIA_CARD_GAP)
            - MEDIA_CARD_GAP;
        let viewport_height = self.media_viewport_height(ui);
        egui::ScrollArea::vertical()
            .id_salt("media.grid")
            .max_height(viewport_height)
            .show(ui, |ui| {
                let (content, _) = ui.allocate_exact_size(
                    Vec2::new(width, content_height.max(viewport_height)),
                    egui::Sense::hover(),
                );
                let background = ui.interact(
                    content,
                    Id::unique("media.marquee"),
                    egui::Sense::click_and_drag(),
                );
                self.remember("media.marquee", &background);
                let mut rects = Vec::with_capacity(rows.len());
                for (index, row) in rows.iter().enumerate() {
                    let column = index % columns;
                    let line = index / columns;
                    let rect = Rect::from_min_size(
                        content.min
                            + Vec2::new(
                                column as f32 * (MEDIA_CARD_WIDTH + MEDIA_CARD_GAP),
                                line as f32 * (MEDIA_CARD_HEIGHT + MEDIA_CARD_GAP),
                            ),
                        Vec2::new(MEDIA_CARD_WIDTH, MEDIA_CARD_HEIGHT),
                    );
                    let response = ui
                        .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                            self.media_asset(ui, row, importing, true)
                        })
                        .inner;
                    if self
                        .media_reveal
                        .is_some_and(|reveal| reveal.asset == row.id)
                    {
                        ui.scroll_to_rect(rect, Some(egui::Align::Center));
                    }
                    if response.clicked() {
                        let modifiers = ui.ctx().input(|input| input.modifiers);
                        self.select_asset(row.id, modifiers);
                    }
                    rects.push((row.id, rect));
                }
                self.media_marquee(ui, content, &background, &rects);
            });
    }
    fn media_list(&mut self, ui: &mut egui::Ui, rows: &[AssetRow], importing: bool) {
        let width = (ui.clip_rect().right() - ui.cursor().min.x).max(0.0);
        let viewport_height = self.media_viewport_height(ui);
        egui::ScrollArea::vertical()
            .id_salt("media.list")
            .max_height(viewport_height)
            .show(ui, |ui| {
                let (content, _) = ui.allocate_exact_size(
                    Vec2::new(
                        width,
                        (rows.len() as f32 * MEDIA_ROW_HEIGHT).max(viewport_height),
                    ),
                    egui::Sense::hover(),
                );
                let background = ui.interact(
                    content,
                    Id::unique("media.marquee"),
                    egui::Sense::click_and_drag(),
                );
                self.remember("media.marquee", &background);
                let mut rects = Vec::with_capacity(rows.len());
                for (index, row) in rows.iter().enumerate() {
                    let rect = Rect::from_min_size(
                        content.min + Vec2::new(0.0, index as f32 * MEDIA_ROW_HEIGHT),
                        Vec2::new(content.width(), MEDIA_ROW_HEIGHT - 2.0),
                    );
                    let response = ui
                        .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                            self.media_asset(ui, row, importing, false)
                        })
                        .inner;
                    if self
                        .media_reveal
                        .is_some_and(|reveal| reveal.asset == row.id)
                    {
                        ui.scroll_to_rect(rect, Some(egui::Align::Center));
                    }
                    if response.clicked() {
                        let modifiers = ui.ctx().input(|input| input.modifiers);
                        self.select_asset(row.id, modifiers);
                    }
                    rects.push((row.id, rect));
                }
                self.media_marquee(ui, content, &background, &rects);
            });
    }
    fn media_marquee(
        &mut self,
        ui: &mut egui::Ui,
        content: Rect,
        background: &egui::Response,
        rects: &[(AssetId, Rect)],
    ) {
        let modifiers = ui.ctx().input(|input| input.modifiers);
        if background.drag_started()
            && self.media_marquee.is_none()
            && let Some(origin) = ui.input(|input| input.pointer.press_origin())
            && !rects.iter().any(|(_, rect)| rect.contains(origin))
        {
            self.media_marquee = Some(MediaMarquee {
                origin,
                before: self.media_selection.clone(),
                additive: modifiers.command || modifiers.ctrl || modifiers.shift,
                moved: false,
            });
        }
        if let Some(marquee) = self.media_marquee.clone()
            && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
        {
            let rect = Rect::from_two_pos(marquee.origin, pointer).intersect(content);
            let mut selected =
                crate::selection::intersections(rect.min, rect.max, rects.iter().copied());
            if marquee.additive {
                selected.extend(&marquee.before);
            }
            self.media_selection = selected;
            if let Some(state) = &mut self.media_marquee {
                state.moved |= marquee.origin.distance(pointer) > 4.0;
            }
            ui.painter().with_clip_rect(content).rect_filled(
                rect,
                0.0,
                self.accent().gamma_multiply(0.15),
            );
            ui.painter().with_clip_rect(content).rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0, self.accent()),
                egui::StrokeKind::Inside,
            );
        }
        if background.drag_stopped()
            && let Some(marquee) = self.media_marquee.take()
        {
            if !marquee.moved {
                self.media_selection.clear();
                self.media_anchor = None;
            }
            self.session.end_merge();
        }
        if background.clicked() {
            self.media_selection.clear();
            self.media_anchor = None;
            self.session.end_merge();
        }
    }
    fn media_asset(
        &mut self,
        ui: &mut egui::Ui,
        row: &AssetRow,
        importing: bool,
        grid: bool,
    ) -> egui::Response {
        let rect = ui.max_rect();
        let response = ui.interact(
            rect,
            Id::unique(("media.asset", row.id.0)),
            egui::Sense::click(),
        );
        self.remember_with_icon(
            format!("media.asset.{}", row.id.0),
            Some(row.icon.name()),
            &response,
        );
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, row.name.as_str()));
        if response.secondary_clicked() && !self.media_selection.contains(&row.id) {
            self.media_selection = [row.id].into_iter().collect();
            self.media_anchor = Some(row.id);
            self.session.end_merge();
        }
        let selected = self.media_selection.contains(&row.id);
        let highlighted = self
            .media_reveal
            .is_some_and(|reveal| reveal.asset == row.id);
        let dark = self.preferences.dark;
        let fill = if selected {
            self.accent()
        } else if response.hovered() {
            theme::hover(dark)
        } else {
            self.control_fill()
        };
        let text = if selected {
            theme::on_accent(dark)
        } else {
            self.text_color()
        };
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 4.0, fill);
        painter.rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(
                if highlighted { 2.0 } else { 1.0 },
                if highlighted {
                    self.accent()
                } else {
                    self.border()
                },
            ),
            egui::StrokeKind::Inside,
        );
        let secondary = if selected {
            theme::on_accent(dark)
        } else {
            self.secondary()
        };
        if grid {
            let thumbnail = Rect::from_min_size(
                rect.min + Vec2::new(4.0, 4.0),
                Vec2::new(rect.width() - 8.0, 58.0),
            );
            painter.rect_filled(thumbnail, 2.0, theme::field(dark));
            self.media_thumbnail(ui, row, thumbnail, 24.0, self.text_color());
            if let Some(duration) = row.duration {
                let label = display_short_time(duration);
                let badge = Rect::from_min_size(
                    thumbnail.right_bottom() - Vec2::new(4.0, 4.0),
                    Vec2::new(0.0, 0.0),
                );
                let galley =
                    painter.layout_no_wrap(label, FontId::proportional(14.0), theme::on_scrim());
                let badge = Rect::from_min_size(
                    badge.min - Vec2::new(galley.size().x + 4.0, galley.size().y + 2.0),
                    galley.size() + Vec2::new(4.0, 2.0),
                );
                painter.rect_filled(badge, 2.0, theme::scrim());
                painter.galley(badge.min + Vec2::new(2.0, 1.0), galley, theme::on_scrim());
            }
            painter.text(
                egui::pos2(rect.left() + 5.0, thumbnail.bottom() + 5.0),
                Align2::LEFT_TOP,
                &row.name,
                FontId::proportional(14.0),
                text,
            );
            painter.text(
                egui::pos2(rect.left() + 5.0, thumbnail.bottom() + 27.0),
                Align2::LEFT_TOP,
                &row.brief,
                FontId::proportional(14.0),
                secondary,
            );
        } else {
            let thumbnail = Rect::from_center_size(
                egui::pos2(rect.left() + 36.0, rect.center().y),
                Vec2::new(64.0, 36.0),
            );
            self.media_thumbnail(ui, row, thumbnail, BUTTON_ICON, text);
            painter.text(
                egui::pos2(rect.left() + 74.0, rect.center().y - 2.0),
                Align2::LEFT_BOTTOM,
                &row.name,
                FontId::proportional(14.0),
                text,
            );
            painter.text(
                egui::pos2(rect.left() + 74.0, rect.center().y + 2.0),
                Align2::LEFT_TOP,
                &row.info,
                FontId::proportional(14.0),
                secondary,
            );
        }
        let status_rect = Rect::from_min_size(
            if grid {
                rect.min + Vec2::splat(6.0)
            } else {
                egui::pos2(rect.right() - 102.0, rect.top() + 3.0)
            },
            Vec2::new((rect.width() - 12.0).min(96.0), 20.0),
        );
        painter.rect_filled(status_rect, 2.0, self.control_fill());
        let status = ui.put(
            status_rect,
            egui::Label::new(
                egui::RichText::new(asset_state_text(self.preferences.locale, row.state))
                    .size(14.0)
                    .color(asset_state_color(row.state, dark)),
            )
            .truncate(),
        );
        self.remember(format!("media.asset.{}.status", row.id.0), &status);
        let ids: Vec<AssetId> = if selected {
            self.media_selection.iter().copied().collect()
        } else {
            vec![row.id]
        };
        let label = if ids.len() == 1 {
            self.tr(Text::DeleteItemOne).to_owned()
        } else {
            catalog::format_count(
                self.preferences.locale,
                ids.len() as u64,
                Text::DeleteItemsOther,
                Text::DeleteItemsOther,
            )
        };
        response.context_menu(|ui| {
            let delete = ui.add_enabled(!importing, egui::Button::new(label));
            self.remember("media.delete", &delete);
            if delete.clicked() {
                self.request_media_delete(ids.clone());
                ui.close();
            }
        });
        response
    }
    fn media_thumbnail(
        &mut self,
        ui: &mut egui::Ui,
        row: &AssetRow,
        rect: Rect,
        fallback_size: f32,
        color: Color32,
    ) {
        if let Some(texture) = self.thumbnail_texture(ui.ctx(), row.id) {
            let size = texture.size_vec2();
            let scale = (rect.width() / size.x).min(rect.height() / size.y);
            let image_rect = Rect::from_center_size(rect.center(), size * scale);
            let response = ui.put(
                image_rect,
                egui::Image::new((texture.id(), image_rect.size())),
            );
            self.remember(format!("media.asset.{}.thumbnail", row.id.0), &response);
        } else {
            icons::tinted(row.icon, fallback_size, color).paint_at(
                ui,
                Rect::from_center_size(rect.center(), Vec2::splat(fallback_size)),
            );
        }
    }
    fn inspector(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if self.text_button(ui, "inspector.project", Text::Project) {
                self.action("inspector.project");
            }
            if self.text_button(ui, "inspector.details", Text::Details) {
                self.action("inspector.details");
            }
        });
        ui.separator();
        ui.add_space(8.0);
        if self.details {
            self.property_focus = None;
            self.section(
                ui,
                "inspector.section.canvas",
                "inspector.canvas",
                Text::Canvas,
                true,
                |editor, ui| {
                    let locale = editor.preferences.locale;
                    let data = editor.session.project().data();
                    let width = catalog::number(locale, u64::from(data.canvas.width));
                    let height = catalog::number(locale, u64::from(data.canvas.height));
                    ui.label(catalog::format(
                        locale,
                        Text::CanvasSize,
                        &[("width", &width), ("height", &height)],
                    ));
                    let numerator = catalog::signed(locale, data.canvas.fps.num());
                    let denominator = catalog::signed(locale, data.canvas.fps.den());
                    ui.label(catalog::format(
                        locale,
                        Text::FrameRate,
                        &[("num", &numerator), ("den", &denominator)],
                    ));
                    let rate = catalog::number(locale, u64::from(data.settings.audio_sample_rate));
                    ui.label(catalog::format(
                        locale,
                        Text::SampleRate,
                        &[("rate", &rate)],
                    ));
                },
            );
            self.section(
                ui,
                "inspector.section.metadata",
                "inspector.metadata",
                Text::Metadata,
                false,
                |editor, ui| {
                    let locale = editor.preferences.locale;
                    let created = catalog::date(locale, editor.session.metadata().created_unix_ms);
                    let modified =
                        catalog::date(locale, editor.session.metadata().modified_unix_ms);
                    ui.label(
                        egui::RichText::new(catalog::format(
                            locale,
                            Text::CreatedDate,
                            &[("date", &created)],
                        ))
                        .small()
                        .color(editor.secondary()),
                    );
                    ui.label(
                        egui::RichText::new(catalog::format(
                            locale,
                            Text::ModifiedDate,
                            &[("date", &modified)],
                        ))
                        .small()
                        .color(editor.secondary()),
                    );
                },
            );
        } else if self.selection.is_empty() {
            self.property_focus = None;
            ui.label(self.tr(Text::EmptyInspector));
        } else if self.selection.len() == 1 {
            self.properties(ui);
        } else {
            self.property_focus = None;
            egui::ScrollArea::vertical()
                .id_salt("inspector.shared")
                .show(ui, |ui| {
                    self.section(
                        ui,
                        "inspector.section.selection",
                        "inspector.selection",
                        Text::Selection,
                        true,
                        |editor, ui| editor.shared_inspector(ui),
                    );
                });
        }
    }
    /// One selected element: its type's tab set, remembered per element type.
    fn properties(&mut self, ui: &mut egui::Ui) {
        let Some(item) = self.selected_item_id() else {
            return;
        };
        let Some((kind, has_audio)) = self.element_kind(item) else {
            return;
        };
        if self.property_selection != self.selection {
            self.property_drafts.clear();
            self.property_focus = None;
            self.property_selection = self.selection.clone();
        }
        let tabs: Vec<PropertiesTab> = kind
            .tabs()
            .iter()
            .copied()
            .filter(|tab| {
                !(*tab == PropertiesTab::Audio && kind == ElementKind::Video && !has_audio)
            })
            .collect();
        let stored = self
            .preferences
            .properties_tabs
            .get(kind.id())
            .and_then(|id| PropertiesTab::from_id(id))
            .filter(|tab| tabs.contains(tab));
        let active = stored.unwrap_or_else(|| kind.default_tab());
        match self.selection_scopes.active(&self.selection) {
            SelectionScope::MaskPoints => {
                ui.label(self.tr(Text::MaskPoints));
            }
            SelectionScope::Keyframes => {
                ui.label(self.tr(Text::Keyframes));
            }
            _ => {}
        }
        let mut clicked = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for tab in &tabs {
                let id = format!("inspector.tab.{}", tab.id());
                let color = if *tab == active {
                    self.accent()
                } else {
                    self.secondary()
                };
                let label = egui::RichText::new(self.tr(tab.label()))
                    .size(14.0)
                    .color(color);
                let response = ui
                    .push_id(id.clone(), |ui| {
                        ui.add(
                            egui::Button::new(label)
                                .frame(false)
                                .wrap_mode(egui::TextWrapMode::Extend)
                                .min_size(Vec2::new(0.0, 32.0)),
                        )
                    })
                    .inner;
                self.remember(&id, &response);
                if *tab == active {
                    ui.painter().line_segment(
                        [response.rect.left_bottom(), response.rect.right_bottom()],
                        egui::Stroke::new(2.0, self.accent()),
                    );
                }
                if response.clicked() {
                    clicked = Some(*tab);
                }
            }
        });
        if let Some(tab) = clicked {
            self.property_focus = None;
            let mut next = self.preferences.properties_tabs.clone();
            next.insert(kind.id().to_owned(), tab.id().to_owned());
            let value = serde_json::to_value(next).map_err(|_| Error::Invalid("tabs"));
            match value.and_then(|value| self.preferences.set("properties_tabs", value)) {
                Ok(()) => {}
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        ui.separator();
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt("inspector.properties")
            .show(ui, |ui| self.properties_tab(ui, item, kind, active));
    }
    fn properties_tab(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        kind: ElementKind,
        tab: PropertiesTab,
    ) {
        match tab {
            PropertiesTab::Text if self.item_value(item, TEXT_SIZE_SCRUB.field).is_some() => {
                for field in shared_fields(self.session.project(), &self.selection) {
                    match field.id {
                        "text_size" => self.scrub_field(ui, item, TEXT_SIZE_SCRUB),
                        "text_color" => self.text_color_field(ui, item),
                        "text_content" | "text_font" | "text_background" | "text_position_x"
                        | "text_position_y" => {
                            self.shared_field_controls(ui, std::iter::once(field));
                        }
                        _ => {}
                    }
                }
            }
            PropertiesTab::Audio if kind == ElementKind::Audio => {
                self.scrub_field(ui, item, AUDIO_GAIN_SCRUB);
                if let Some(field) = shared_fields(self.session.project(), &self.selection)
                    .into_iter()
                    .find(|field| field.id == "audio_muted")
                {
                    self.audio_mute_control(ui, &field);
                }
            }
            PropertiesTab::Blending => {
                self.blend_control(ui, item);
                self.scrub_field(ui, item, OPACITY_SCRUB);
            }
            PropertiesTab::Effects
                if matches!(
                    kind,
                    ElementKind::Video
                        | ElementKind::Image
                        | ElementKind::Sticker
                        | ElementKind::Graphic
                ) =>
            {
                self.effects_stack(ui, item);
            }
            _ => self.properties_stub(ui),
        }
    }
    fn effects_stack(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let effects = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)
            .map_or_else(Vec::new, |candidate| candidate.effects.clone());
        if effects.is_empty() {
            let response = ui.label(
                egui::RichText::new(self.tr(Text::EffectEmptyStack))
                    .small()
                    .color(self.secondary()),
            );
            self.remember("effects.stack.empty", &response);
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(self.tr(Text::EffectDropHint))
                    .small()
                    .color(self.secondary()),
            );
            return;
        }
        let mut rows: Vec<(String, Rect)> = Vec::new();
        let mut released = false;
        for effect in &effects {
            let mut dropped = false;
            let row = ui
                .scope(|ui| {
                    ui.horizontal(|ui| {
                        let handle_id = format!("effects.row.{}.handle", effect.id);
                        let (rect, handle) = ui.allocate_exact_size(
                            Vec2::new(28.0, 26.0),
                            egui::Sense::click_and_drag(),
                        );
                        if handle.hovered() || handle.dragged() {
                            ui.painter().rect_filled(rect, 4.0, self.control_fill());
                        }
                        icons::tinted(Icon::ArrowsInLineHorizontal, BUTTON_ICON, self.secondary())
                            .paint_at(
                                ui,
                                Rect::from_center_size(rect.center(), Vec2::splat(BUTTON_ICON)),
                            );
                        let handle = handle.on_hover_text(self.tr(Text::EffectReorder));
                        self.remember_with_icon(
                            handle_id,
                            Some(Icon::ArrowsInLineHorizontal.name()),
                            &handle,
                        );
                        if handle.drag_started() {
                            self.effect_reorder = Some(EffectReorder {
                                item,
                                effect: effect.id.clone(),
                            });
                        }
                        if handle.drag_stopped() {
                            dropped = true;
                        }
                        let label_color = if effect.enabled {
                            self.text_color()
                        } else {
                            self.secondary()
                        };
                        let name = effect_definition(&effect.params)
                            .map_or("Effect", |definition| self.effect_name(definition));
                        ui.label(egui::RichText::new(name).size(14.0).color(label_color));
                        if !effect.enabled {
                            let response = ui.label(
                                egui::RichText::new(self.tr(Text::EffectDisabled))
                                    .small()
                                    .color(self.secondary()),
                            );
                            self.remember(format!("effects.row.{}.disabled", effect.id), &response);
                        }
                        let (icon, label) = if effect.enabled {
                            (Icon::Eye, Text::EffectDisable)
                        } else {
                            (Icon::EyeSlash, Text::EffectEnable)
                        };
                        let enabled = self.icon_only(
                            ui,
                            &format!("effects.toggle.{}", effect.id),
                            icon,
                            self.tr(label),
                            None,
                            Some(effect.enabled),
                            true,
                        );
                        if enabled {
                            self.toggle_effect(item, &effect.id, !effect.enabled);
                        }
                        let remove = self.tr(Text::EffectRemove);
                        if self.icon_only(
                            ui,
                            &format!("effects.remove.{}", effect.id),
                            Icon::Trash,
                            remove,
                            None,
                            None,
                            true,
                        ) {
                            self.remove_effect(item, &effect.id);
                        }
                    });
                    if let Some(definition) = effect_definition(&effect.params) {
                        for parameter in definition.params {
                            if self
                                .effect_param_value(item, &effect.id, parameter.key)
                                .is_some()
                            {
                                self.effect_scrub_field(ui, item, &effect.id, parameter);
                            }
                        }
                    }
                })
                .response
                .rect;
            let full = Rect::from_min_max(
                egui::pos2(ui.max_rect().left(), row.top()),
                egui::pos2(ui.max_rect().right(), row.bottom()),
            );
            rows.push((effect.id.clone(), full));
            released |= dropped;
        }
        if released {
            let pointer = ui.input(|input| {
                input
                    .pointer
                    .interact_pos()
                    .or_else(|| input.pointer.latest_pos())
            });
            let target = pointer.and_then(|position| {
                rows.iter()
                    .find(|(id, rect)| {
                        self.effect_reorder
                            .as_ref()
                            .is_none_or(|drag| drag.effect != *id)
                            && rect.contains(position)
                    })
                    .map(|(id, _)| id.clone())
            });
            if let Some(drag) = self.effect_reorder.take()
                && let Some(target) = target
            {
                self.reorder_effect(drag.item, &drag.effect, &target);
            }
        } else if self.effect_reorder.is_some() && ui.input(|input| input.pointer.any_released()) {
            self.effect_reorder = None;
        }
    }
    fn effect_scrub_field(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        effect: &str,
        parameter: &project::EffectParamDefinition,
    ) {
        let Some(current) = self.effect_param_value(item, effect, parameter.key) else {
            return;
        };
        let spec = effect_scrub_spec(parameter);
        ui.add_space(8.0);
        let label = match parameter.key {
            "intensity" => self.tr(Text::EffectIntensity),
            _ => parameter.name,
        };
        let caption = ui.label(label);
        let draft_key = format!("effects.{effect}.param.{}", parameter.key);
        ui.horizontal(|ui| {
            let handle_id = format!("effects.{effect}.param.{}.scrub", parameter.key);
            let (rect, handle) =
                ui.allocate_exact_size(Vec2::new(28.0, 26.0), egui::Sense::click_and_drag());
            if handle.hovered() || handle.dragged() {
                ui.painter().rect_filled(rect, 4.0, self.control_fill());
            }
            icons::tinted(Icon::ArrowsHorizontal, BUTTON_ICON, self.secondary()).paint_at(
                ui,
                Rect::from_center_size(rect.center(), Vec2::splat(BUTTON_ICON)),
            );
            let handle = handle.on_hover_text(self.tr(Text::DragToAdjust));
            self.remember_with_icon(
                handle_id.clone(),
                Some(Icon::ArrowsHorizontal.name()),
                &handle,
            );
            if handle.drag_started() {
                self.start_effect_scrub(item, effect, parameter.key, current);
            }
            if handle.dragged()
                && let Some(mut scrub) = self.scrub.take()
            {
                let target = ScrubTarget::Effect {
                    effect: effect.to_owned(),
                    key: parameter.key.to_owned(),
                };
                if scrub.target == target {
                    let steps = f64::from(handle.total_drag_delta().map_or(0.0, |delta| delta.x))
                        / f64::from(spec.pixels_per_step);
                    let value = spec.quantize(scrub.start + steps * spec.step);
                    if value != scrub.last {
                        match self.set_effect_param(item, effect, parameter.key, value) {
                            Ok(()) => scrub.last = value,
                            Err(error) => self.error = Some(error.to_string()),
                        }
                    }
                }
                self.scrub = Some(scrub);
            }
            if handle.drag_stopped() {
                self.finish_scrub(true);
            }
            let width = ui.available_width().max(60.0);
            let mut next = self
                .property_drafts
                .get(&draft_key)
                .cloned()
                .unwrap_or_else(|| spec.format(current));
            let response = ui
                .add_sized(
                    [width, 26.0],
                    egui::TextEdit::singleline(&mut next)
                        .char_limit(32)
                        .background_color(self.control_fill())
                        .id(Id::unique(&draft_key))
                        .hint_text(
                            egui::RichText::new(self.tr(Text::EmptyValue)).color(self.secondary()),
                        ),
                )
                .labelled_by(caption.id);
            self.remember(draft_key.clone(), &response);
            if response.has_focus() {
                self.property_focus = Some(draft_key.clone());
            }
            if response.changed() {
                self.property_drafts.insert(draft_key.clone(), next);
            }
            let editing =
                response.has_focus() || self.property_focus.as_deref() == Some(draft_key.as_str());
            if editing && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.property_drafts.remove(&draft_key);
                self.property_focus = None;
                response.surrender_focus();
                return;
            }
            let enter = response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if enter || response.lost_focus() {
                self.property_focus = None;
                if let Some(draft) = self.property_drafts.remove(&draft_key) {
                    let Ok(parsed) = draft.trim().parse::<f64>() else {
                        self.error = Some(Error::Invalid("scrub value").to_string());
                        return;
                    };
                    if !parsed.is_finite() {
                        self.error = Some(Error::Invalid("scrub value").to_string());
                        return;
                    }
                    let value = spec.quantize(parsed);
                    if let Err(error) = self.set_effect_param(item, effect, parameter.key, value) {
                        self.error = Some(error.to_string());
                    }
                }
            }
        });
    }
    fn properties_stub(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new(self.tr(Text::PanelStub))
                .small()
                .color(self.secondary()),
        );
    }
    fn selected_item_id(&self) -> Option<ItemId> {
        if self.selection.len() != 1 {
            return None;
        }
        self.selection.iter().next().copied()
    }
    fn element_kind(&self, id: ItemId) -> Option<(ElementKind, bool)> {
        let data = self.session.project().data();
        let (track, item) = data.scene().timeline.iter().find_map(|track| {
            track
                .items
                .iter()
                .find(|item| item.id == id)
                .map(|item| (track, item))
        })?;
        let kind = match (&item.content, track.kind) {
            (ItemParams::Text(_) | ItemParams::Caption(_), _) => ElementKind::Text,
            (ItemParams::Audio(_), _) => ElementKind::Audio,
            (_, TrackKind::Sticker) => ElementKind::Sticker,
            (_, TrackKind::Effect) => ElementKind::Effect,
            (_, TrackKind::Filter | TrackKind::Adjustment) => ElementKind::Graphic,
            (ItemParams::Overlay(_), _) => ElementKind::Graphic,
            (ItemParams::Video(_), _) => {
                let Some(asset) = item
                    .source_asset
                    .and_then(|asset| data.assets.iter().find(|candidate| candidate.id() == asset))
                else {
                    return Some((ElementKind::Graphic, false));
                };
                let video = asset
                    .probe()
                    .streams
                    .iter()
                    .find(|stream| stream.kind == StreamKind::Video);
                match video {
                    Some(stream) if stream.frame_count == Some(1) => ElementKind::Image,
                    Some(_) => ElementKind::Video,
                    None => ElementKind::Image,
                }
            }
        };
        let has_audio = kind == ElementKind::Video
            && item.source_asset.is_some_and(|asset| {
                data.assets.iter().any(|candidate| {
                    candidate.id() == asset
                        && candidate
                            .probe()
                            .streams
                            .iter()
                            .any(|stream| stream.kind == StreamKind::Audio)
                })
            });
        Some((kind, has_audio))
    }
    fn effect_refusal(&self, id: ItemId) -> Option<Text> {
        let data = self.session.project().data();
        let track = data
            .scene()
            .timeline
            .iter()
            .find(|track| track.items.iter().any(|item| item.id == id))?;
        let (kind, _) = self.element_kind(id)?;
        if track.locked {
            return Some(Text::EffectLocked);
        }
        if kind == ElementKind::Audio {
            return Some(Text::EffectAudioUnsupported);
        }
        if !matches!(
            kind,
            ElementKind::Video | ElementKind::Image | ElementKind::Sticker | ElementKind::Graphic
        ) {
            return Some(Text::EffectKindUnsupported);
        }
        None
    }
    fn add_effect_to_item(&mut self, item: ItemId, kind: &str) {
        if let Some(reason) = self.effect_refusal(item) {
            let text = self.tr(reason).to_owned();
            self.set_toast(&text, None);
            return;
        }
        let Some(definition) = project::effect_definitions()
            .iter()
            .find(|definition| definition.kind == kind)
        else {
            let text = self.tr(Text::EffectKindUnsupported).to_owned();
            self.set_toast(&text, None);
            return;
        };
        let Some(params) = effect_params_from_definition(definition) else {
            let text = self.tr(Text::EffectKindUnsupported).to_owned();
            self.set_toast(&text, None);
            return;
        };
        let Some(effect_id) = self.next_effect_id(item, kind) else {
            let text = self.tr(Text::EffectLimit).to_owned();
            self.set_toast(&text, None);
            return;
        };
        let effect = project::EffectInstance {
            id: effect_id,
            enabled: true,
            params,
        };
        let before = self.session.project().clone();
        match self.session.execute(
            "effects.add",
            serde_json::json!({"item": item, "effect": effect}),
        ) {
            Ok(_) => {
                self.select_single(item);
                let text = self.tr(Text::EffectAdded).to_owned();
                let name = self.effect_name(definition).to_owned();
                self.set_toast(&text, Some(name));
            }
            Err(error) => {
                let text = if matches!(
                    &error,
                    engine::EditError::Project(project::Error::TooLarge(_))
                ) {
                    self.tr(Text::EffectLimit).to_owned()
                } else {
                    error_detail(&error)
                };
                self.set_toast(&text, None);
            }
        }
        if !Arc::ptr_eq(&before, self.session.project()) {
            self.sync_preview();
        }
    }
    fn next_effect_id(&self, item: ItemId, kind: &str) -> Option<String> {
        let effects = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)?
            .effects
            .iter()
            .map(|effect| effect.id.as_str())
            .collect::<BTreeSet<_>>();
        (1..=256).find_map(|index| {
            let candidate = if index == 1 {
                kind.to_owned()
            } else {
                format!("{kind}{index}")
            };
            (candidate.len() <= 64 && !effects.contains(candidate.as_str())).then_some(candidate)
        })
    }
    fn remove_effect(&mut self, item: ItemId, effect: &str) {
        let before = self.session.project().clone();
        match self.session.execute(
            "effects.remove",
            serde_json::json!({"item": item, "effect": effect}),
        ) {
            Ok(_) => {}
            Err(error) => self.error = Some(error.to_string()),
        }
        if !Arc::ptr_eq(&before, self.session.project()) {
            self.sync_preview();
        }
    }
    fn toggle_effect(&mut self, item: ItemId, effect: &str, enabled: bool) {
        let before = self.session.project().clone();
        match self.session.execute(
            "effects.toggle",
            serde_json::json!({"item": item, "effect": effect, "enabled": enabled}),
        ) {
            Ok(_) => {}
            Err(error) => self.error = Some(error.to_string()),
        }
        if !Arc::ptr_eq(&before, self.session.project()) {
            self.sync_preview();
        }
    }
    fn reorder_effect(&mut self, item: ItemId, effect: &str, target_effect: &str) {
        let Some(current) = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)
            .map(|candidate| {
                candidate
                    .effects
                    .iter()
                    .map(|effect| effect.id.clone())
                    .collect::<Vec<_>>()
            })
        else {
            return;
        };
        let Some(from) = current.iter().position(|id| id == effect) else {
            let text = self.tr(Text::EffectMissing).to_owned();
            self.set_toast(&text, None);
            return;
        };
        let Some(target) = current.iter().position(|id| id == target_effect) else {
            return;
        };
        let mut order = current.clone();
        let moved = order.remove(from);
        order.insert(target, moved);
        if order == current {
            return;
        }
        let before_project = self.session.project().clone();
        match self.session.execute(
            "effects.reorder",
            serde_json::json!({"item": item, "order": order}),
        ) {
            Ok(_) => {}
            Err(error) => self.error = Some(error.to_string()),
        }
        if !Arc::ptr_eq(&before_project, self.session.project()) {
            self.sync_preview();
        }
    }
    fn set_effect_param(
        &mut self,
        item: ItemId,
        effect: &str,
        key: &str,
        value: f64,
    ) -> Result<()> {
        self.session.set_selection(self.selection.clone())?;
        self.session.execute(
            "effects.update_param",
            serde_json::json!({"item": item, "effect": effect, "key": key, "value": value}),
        )?;
        self.sync_preview();
        Ok(())
    }
    fn effect_param_value(&self, item: ItemId, effect: &str, key: &str) -> Option<f64> {
        let effect = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)?
            .effects
            .iter()
            .find(|candidate| candidate.id == effect)?;
        match (&effect.params, key) {
            (project::EffectParams::Blur { intensity }, "intensity") => Some(*intensity),
            _ => None,
        }
    }
    fn blend_control(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(current) = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)
            .map(|item| item.blend_mode)
        else {
            return;
        };
        ui.add_space(12.0);
        let caption = ui.label(self.tr(Text::BlendMode));
        let label = self.tr(catalog::blend_label(current));
        let response = ui
            .scope(|ui| {
                ui.spacing_mut().interact_size.y = 26.0;
                egui::ComboBox::from_id_salt(Id::unique("inspector.blendMode"))
                    .selected_text(label)
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        for mode in project::BlendMode::ALL {
                            let response = ui.selectable_label(
                                current == mode,
                                self.tr(catalog::blend_label(mode)),
                            );
                            self.remember(format!("inspector.blendMode.{}", mode.id()), &response);
                            if response.clicked() {
                                if let Err(error) =
                                    self.set_item_field(item, "blendMode", mode.id())
                                {
                                    self.error = Some(error.to_string());
                                }
                                ui.close();
                            }
                        }
                    })
                    .response
                    .labelled_by(caption.id)
            })
            .inner;
        self.remember("inspector.blendMode", &response);
    }
    fn text_color_field(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(value) = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)
            .and_then(|item| match &item.content {
                ItemParams::Text(params) => {
                    let [r, g, b, a] = params.color.0;
                    Some(format!("#{r:02X}{g:02X}{b:02X}{a:02X}"))
                }
                _ => None,
            })
        else {
            return;
        };
        ui.add_space(12.0);
        let caption = ui.label(self.tr(Text::TextColor));
        let draft = self
            .property_drafts
            .get("text_color")
            .cloned()
            .unwrap_or(value);
        let mut next = draft;
        let response = ui
            .add_sized(
                [ui.available_width(), 26.0],
                egui::TextEdit::singleline(&mut next)
                    .char_limit(64)
                    .background_color(self.control_fill())
                    .id(Id::unique("inspector.text_color"))
                    .hint_text(
                        egui::RichText::new(self.tr(Text::EmptyValue)).color(self.secondary()),
                    ),
            )
            .labelled_by(caption.id);
        self.remember("inspector.text_color", &response);
        if response.has_focus() {
            self.property_focus = Some("text_color".to_owned());
        }
        if response.changed() {
            self.property_drafts
                .insert("text_color".to_owned(), next.clone());
        }
        let editing = response.has_focus() || self.property_focus.as_deref() == Some("text_color");
        if editing && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.property_drafts.remove("text_color");
            self.property_focus = None;
            response.surrender_focus();
            return;
        }
        let enter = response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if enter || response.lost_focus() {
            self.property_focus = None;
            if let Some(draft) = self.property_drafts.remove("text_color")
                && let Err(error) = self.set_item_field(item, "text_color", &draft)
            {
                self.error = Some(error.to_string());
            }
        }
    }
    /// A numeric field whose left icon scrubs, whose text commits on Enter or
    /// blur, and whose gesture is one engine undo step from preview to release.
    fn scrub_field(&mut self, ui: &mut egui::Ui, item: ItemId, spec: ScrubSpec) {
        let Some(current) = self.item_value(item, spec.field) else {
            return;
        };
        ui.add_space(12.0);
        let caption = ui.label(self.tr(spec.label));
        ui.horizontal(|ui| {
            let handle_id = format!("inspector.{}.scrub", spec.field);
            let (rect, handle) =
                ui.allocate_exact_size(Vec2::new(28.0, 26.0), egui::Sense::click_and_drag());
            if handle.hovered() || handle.dragged() {
                ui.painter().rect_filled(rect, 4.0, self.control_fill());
            }
            icons::tinted(Icon::ArrowsHorizontal, BUTTON_ICON, self.secondary()).paint_at(
                ui,
                Rect::from_center_size(rect.center(), Vec2::splat(BUTTON_ICON)),
            );
            let handle = handle.on_hover_text(self.tr(Text::DragToAdjust));
            self.remember_with_icon(
                handle_id.clone(),
                Some(Icon::ArrowsHorizontal.name()),
                &handle,
            );
            if handle.drag_started() {
                self.start_scrub(item, spec, current);
            }
            if handle.dragged()
                && let Some(mut scrub) = self.scrub.take()
            {
                if scrub.target == ScrubTarget::Item(spec.field) {
                    let steps = f64::from(handle.total_drag_delta().map_or(0.0, |delta| delta.x))
                        / f64::from(spec.pixels_per_step);
                    let value = spec.quantize(scrub.start + steps * spec.step);
                    if value != scrub.last {
                        match self.set_item_field(item, spec.field, &spec.format(value)) {
                            Ok(()) => scrub.last = value,
                            Err(error) => self.error = Some(error.to_string()),
                        }
                    }
                }
                self.scrub = Some(scrub);
            }
            if handle.drag_stopped() {
                self.finish_scrub(true);
            }
            let width =
                (ui.available_width() - if spec.suffix.is_empty() { 0.0 } else { 30.0 }).max(60.0);
            let id = format!("inspector.{}", spec.field);
            let mut next = self
                .property_drafts
                .get(spec.field)
                .cloned()
                .unwrap_or_else(|| spec.format(current));
            let response = ui
                .add_sized(
                    [width, 26.0],
                    egui::TextEdit::singleline(&mut next)
                        .char_limit(32)
                        .background_color(self.control_fill())
                        .id(Id::unique(&id))
                        .hint_text(
                            egui::RichText::new(self.tr(Text::EmptyValue)).color(self.secondary()),
                        ),
                )
                .labelled_by(caption.id);
            self.remember(id.clone(), &response);
            if response.has_focus() {
                self.property_focus = Some(spec.field.to_owned());
            }
            if response.changed() {
                self.property_drafts.insert(spec.field.to_owned(), next);
            }
            if !spec.suffix.is_empty() {
                ui.label(
                    egui::RichText::new(spec.suffix)
                        .small()
                        .color(self.secondary()),
                );
            }
            let editing =
                response.has_focus() || self.property_focus.as_deref() == Some(spec.field);
            if editing && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.property_drafts.remove(spec.field);
                self.property_focus = None;
                response.surrender_focus();
                return;
            }
            let enter = response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if enter || response.lost_focus() {
                self.property_focus = None;
                if let Some(draft) = self.property_drafts.remove(spec.field) {
                    let Ok(parsed) = draft.trim().parse::<f64>() else {
                        self.error = Some(Error::Invalid("scrub value").to_string());
                        return;
                    };
                    if !parsed.is_finite() {
                        self.error = Some(Error::Invalid("scrub value").to_string());
                        return;
                    }
                    let value = spec.quantize(parsed);
                    if let Err(error) = self.set_item_field(item, spec.field, &spec.format(value)) {
                        self.error = Some(error.to_string());
                    }
                }
            }
        });
    }
    fn item_value(&self, item: ItemId, field: &str) -> Option<f64> {
        let item = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)?;
        match (field, &item.content) {
            ("opacity", content) if !matches!(content, ItemParams::Audio(_)) => {
                let path = project::AnimationPath::new("opacity").ok()?;
                Some(
                    match item.animations.value_at(&path, Tick::ZERO).ok().flatten() {
                        Some(project::ParamValue::Scalar(value)) => value.clamp(0.0, 1.0),
                        _ => 1.0,
                    },
                )
            }
            ("text_size", ItemParams::Text(params)) => Some(f64::from(params.font_size)),
            ("audio_gain", ItemParams::Audio(params)) => Some(f64::from(params.gain_db)),
            _ => None,
        }
    }
    fn start_scrub(&mut self, item: ItemId, spec: ScrubSpec, value: f64) {
        self.start_scrub_target(item, ScrubTarget::Item(spec.field), value);
    }
    fn start_effect_scrub(&mut self, item: ItemId, effect: &str, key: &str, value: f64) {
        self.start_scrub_target(
            item,
            ScrubTarget::Effect {
                effect: effect.to_owned(),
                key: key.to_owned(),
            },
            value,
        );
    }
    fn start_scrub_target(&mut self, item: ItemId, target: ScrubTarget, value: f64) {
        if self.scrub.is_some() {
            self.finish_scrub(false);
        }
        self.session.end_merge();
        if self.session.transaction_active()
            && let Err(error) = self.session.cancel_gesture()
        {
            self.error = Some(error.to_string());
            return;
        }
        if let Err(error) = self
            .session
            .set_selection(self.selection.clone())
            .and_then(|_| self.session.begin_gesture())
        {
            self.error = Some(error.to_string());
            return;
        }
        self.property_drafts.remove(&target.draft_key());
        self.property_focus = None;
        self.scrub = Some(ScrubDrag {
            item,
            target,
            start: value,
            last: value,
        });
    }
    fn finish_scrub(&mut self, commit: bool) {
        let Some(scrub) = self.scrub.take() else {
            return;
        };
        let changed = scrub.last != scrub.start;
        let result = if commit && changed {
            self.session.commit_gesture()
        } else {
            self.session.cancel_gesture()
        };
        if let Err(error) = result {
            self.error = Some(error.to_string());
        }
        self.property_drafts.remove(&scrub.target.draft_key());
        if changed {
            self.sync_preview();
        }
    }
    fn shared_inspector(&mut self, ui: &mut egui::Ui) {
        if self.property_selection != self.selection {
            self.property_drafts.clear();
            self.property_selection = self.selection.clone();
        }
        ui.label(catalog::format_count(
            self.preferences.locale,
            self.selection.len() as u64,
            Text::SelectedCountOne,
            Text::SelectedCountOther,
        ));
        match self.selection_scopes.active(&self.selection) {
            SelectionScope::MaskPoints => {
                ui.label(self.tr(Text::MaskPoints));
            }
            SelectionScope::Keyframes => {
                ui.label(self.tr(Text::Keyframes));
            }
            _ => {}
        }
        let fields = shared_fields(self.session.project(), &self.selection);
        if fields.is_empty() {
            ui.label(self.tr(Text::NoSharedFields));
        }
        self.shared_field_controls(ui, fields);
    }
    fn shared_field_controls(
        &mut self,
        ui: &mut egui::Ui,
        fields: impl IntoIterator<Item = crate::selection::SharedField>,
    ) {
        for field in fields {
            if field.id == "audio_muted" {
                self.audio_mute_control(ui, &field);
                continue;
            }
            if field.id == "blendMode" {
                ui.add_space(12.0);
                let caption = ui.label(self.tr(Text::BlendMode));
                let current = field.value.as_ref().and_then(|v| {
                    serde_json::from_value::<project::BlendMode>(serde_json::json!(v)).ok()
                });
                let label = current
                    .map(|m| self.tr(catalog::blend_label(m)))
                    .unwrap_or_else(|| self.tr(Text::EmptyValue));
                let response = ui
                    .scope(|ui| {
                        ui.spacing_mut().interact_size.y = 26.0;
                        egui::ComboBox::from_id_salt(Id::unique("inspector.blendMode"))
                            .selected_text(label)
                            .width(ui.available_width())
                            .show_ui(ui, |ui| {
                                for mode in project::BlendMode::ALL {
                                    let response = ui.selectable_label(
                                        current == Some(mode),
                                        self.tr(catalog::blend_label(mode)),
                                    );
                                    self.remember(
                                        format!("inspector.blendMode.{}", mode.id()),
                                        &response,
                                    );
                                    if response.clicked() {
                                        if let Err(error) =
                                            self.set_shared_field("blendMode", mode.id())
                                        {
                                            self.error = Some(error.to_string());
                                        }
                                        ui.close();
                                    }
                                }
                            })
                            .response
                            .labelled_by(caption.id)
                    })
                    .inner;
                self.remember("inspector.blendMode", &response);
                continue;
            }
            ui.add_space(8.0);
            if field.id == "text_font" {
                let current = field
                    .value
                    .clone()
                    .unwrap_or_else(|| self.tr(Text::EmptyValue).to_owned());
                let mut chosen = None;
                let caption = ui.label(self.tr(Text::TextFont));
                let response = ui
                    .scope(|ui| {
                        ui.spacing_mut().interact_size.y = 26.0;
                        egui::ComboBox::from_id_salt(Id::unique("inspector.text_font"))
                            .selected_text(current)
                            .width(ui.available_width())
                            .show_ui(ui, |ui| {
                                for (family, _) in text::families() {
                                    let selected = field.value.as_deref() == Some(family.as_str());
                                    let response = ui.selectable_label(selected, family.as_str());
                                    self.remember(
                                        format!("inspector.text_font.{family}"),
                                        &response,
                                    );
                                    if response.clicked() {
                                        chosen = Some(family);
                                    }
                                }
                            })
                            .response
                            .labelled_by(caption.id)
                    })
                    .inner;
                self.remember("inspector.text_font", &response);
                if let Some(font) = chosen
                    && let Err(error) = self.set_shared_field("text_font", &font)
                {
                    self.error = Some(error.to_string());
                }
                self.compatible_count(ui, field.compatible);
                continue;
            }
            if field.id == "text_background" {
                let enabled = field.value.as_deref().is_some_and(|value| value != "none");
                let mut checked = enabled;
                let response = ui.add(
                    egui::Checkbox::new(&mut checked, self.tr(Text::TextBackground))
                        .indeterminate(field.value.is_none()),
                );
                self.remember("inspector.text_background", &response);
                if response.changed() {
                    let color = field
                        .value
                        .as_deref()
                        .filter(|value| *value != "none")
                        .unwrap_or("#000000A0");
                    let value = if checked { color } else { "none" };
                    if let Err(error) = self.set_shared_field("text_background", value) {
                        self.error = Some(error.to_string());
                    }
                }
                if enabled {
                    let id = "inspector.text_background_color";
                    ui.label(self.tr(Text::BackgroundColor));
                    let mut value = self
                        .property_drafts
                        .get("text_background_color")
                        .cloned()
                        .or_else(|| field.value.clone())
                        .unwrap_or_default();
                    let response = ui.add_sized(
                        [ui.available_width(), 26.0],
                        egui::TextEdit::singleline(&mut value)
                            .char_limit(64)
                            .background_color(self.control_fill())
                            .id(Id::unique(id))
                            .hint_text(
                                egui::RichText::new(self.tr(Text::EmptyValue))
                                    .color(self.secondary()),
                            ),
                    );
                    self.remember(id, &response);
                    if response.changed() {
                        self.property_drafts
                            .insert("text_background_color".to_owned(), value.clone());
                    }
                    if (response.lost_focus()
                        || (response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))))
                        && self.property_drafts.contains_key("text_background_color")
                    {
                        let result = self.set_shared_field("text_background_color", &value);
                        self.property_drafts.remove("text_background_color");
                        if let Err(e) = result {
                            self.error = Some(e.to_string());
                        }
                    }
                }
                self.compatible_count(ui, field.compatible);
                continue;
            }
            let (label, id) = match field.id {
                "opacity" => (Text::Opacity, "inspector.opacity"),
                "audio_gain" => (Text::AudioGain, "inspector.audio_gain"),
                "text_content" => (Text::TextContent, "inspector.text_content"),
                "text_size" => (Text::TextSize, "inspector.text_size"),
                "text_background" => (Text::BackgroundColor, "inspector.text_background_color"),
                "text_position_x" => (Text::TextPositionX, "inspector.text_position_x"),
                "text_position_y" => (Text::TextPositionY, "inspector.text_position_y"),
                _ => (Text::TextColor, "inspector.text_color"),
            };
            let caption = ui.label(self.tr(label));
            let mut value = self
                .property_drafts
                .get(field.id)
                .cloned()
                .unwrap_or_else(|| field.value.clone().unwrap_or_default());
            let multiline = field.id == "text_content";
            let response = if multiline {
                ui.add_sized(
                    [ui.available_width(), 48.0],
                    egui::TextEdit::multiline(&mut value)
                        .char_limit(4096)
                        .background_color(self.control_fill())
                        .id(Id::unique(id))
                        .hint_text(
                            egui::RichText::new(self.tr(Text::EmptyValue)).color(self.secondary()),
                        ),
                )
            } else {
                ui.add_sized(
                    [ui.available_width(), 26.0],
                    egui::TextEdit::singleline(&mut value)
                        .char_limit(64)
                        .background_color(self.control_fill())
                        .id(Id::unique(id))
                        .hint_text(
                            egui::RichText::new(self.tr(Text::EmptyValue)).color(self.secondary()),
                        ),
                )
            }
            .labelled_by(caption.id);
            self.remember(id, &response);
            if response.changed() {
                self.property_drafts
                    .insert(field.id.to_owned(), value.clone());
            }
            self.compatible_count(ui, field.compatible);
            let commit_key = response.has_focus()
                && ui.input(|i| {
                    i.key_pressed(egui::Key::Enter) && (!multiline || i.modifiers.command)
                });
            if (response.lost_focus() || commit_key) && self.property_drafts.contains_key(field.id)
            {
                let result = self.set_shared_field(field.id, &value);
                self.property_drafts.remove(field.id);
                if let Err(e) = result {
                    self.error = Some(e.to_string());
                }
            }
        }
    }
    fn audio_mute_control(&mut self, ui: &mut egui::Ui, field: &crate::selection::SharedField) {
        ui.add_space(12.0);
        let mut muted = field.value.as_deref() == Some("true");
        let response = ui.add(
            egui::Checkbox::new(&mut muted, self.tr(Text::AudioMuted))
                .indeterminate(field.value.is_none()),
        );
        self.remember("inspector.audio_muted", &response);
        if response.changed()
            && let Err(error) = self.set_shared_field("audio_muted", &muted.to_string())
        {
            self.error = Some(error.to_string());
        }
        if self.selection.len() > 1 {
            self.compatible_count(ui, field.compatible);
        }
    }
    fn compatible_count(&self, ui: &mut egui::Ui, compatible: usize) {
        let compatible = catalog::number(self.preferences.locale, compatible as u64);
        let total = catalog::number(self.preferences.locale, self.selection.len() as u64);
        ui.label(
            egui::RichText::new(catalog::format(
                self.preferences.locale,
                Text::CompatibleCount,
                &[("compatible", &compatible), ("total", &total)],
            ))
            .small()
            .color(self.secondary()),
        );
    }
    /// The inspector and automation use the same registered atomic field edit.
    pub fn set_shared_field(&mut self, field: &str, value: &str) -> Result<()> {
        let patch = item_patch(field, value)?;
        self.session.set_selection(self.selection.clone())?;
        self.session.execute(
            "timeline.set_item_param_many",
            serde_json::json!({"items":self.selection,"patch":patch}),
        )?;
        self.sync_preview();
        Ok(())
    }
    /// One element's committed field edit, through the singular registered command.
    pub fn set_item_field(&mut self, item: ItemId, field: &str, value: &str) -> Result<()> {
        let patch = item_patch(field, value)?;
        self.session.set_selection(self.selection.clone())?;
        self.session.execute(
            "timeline.set_item_param",
            serde_json::json!({"item":item,"patch":patch}),
        )?;
        self.sync_preview();
        Ok(())
    }
    /// Shared transport and preview surface for dedicated review windows.
    pub fn preview_ui(&mut self, ui: &mut egui::Ui) {
        self.preview(ui);
    }
    fn preview(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new(self.tr(Text::Preview))
                .size(16.0)
                .strong(),
        );
        ui.add_space(12.0);
        let mut footer_height = 86.0;
        if !self.playback.device_available() {
            footer_height += 24.0;
        }
        if self.error.is_some() || self.playback.error().is_some() {
            footer_height += 80.0;
        }
        let area = Rect::from_min_size(
            ui.cursor().min,
            Vec2::new(
                ui.available_width(),
                (ui.available_height() - footer_height).max(0.0),
            ),
        );
        ui.painter()
            .rect_filled(area, 0.0, theme::preview_backdrop());
        if let Some(frame) = self
            .playback
            .frame()
            .filter(|_| self.session.project().duration() > Tick::ZERO)
        {
            let key = (frame.epoch, frame.at);
            if self.texture_key != Some(key) {
                if let Some(texture) = &mut self.texture {
                    texture.set(frame.image.clone(), egui::TextureOptions::LINEAR);
                } else {
                    self.texture = Some(ui.ctx().load_texture(
                        "preview.frame",
                        frame.image.clone(),
                        egui::TextureOptions::LINEAR,
                    ));
                }
                self.texture_key = Some(key);
                self.texture_owner = Some(frame.clone());
            }
            if let Some(texture) = &self.texture {
                let size = texture.size_vec2();
                let size = size * (area.width() / size.x).min(area.height() / size.y);
                ui.painter().image(
                    texture.id(),
                    Rect::from_center_size(area.center(), size),
                    Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    theme::image_tint(),
                );
            }
        } else {
            self.texture = None;
            self.texture_key = None;
            self.texture_owner = None;
            let label = if self.playback.error().is_some() {
                Text::Unavailable
            } else if self.session.project().duration() == Tick::ZERO {
                Text::EmptyPreview
            } else {
                Text::Loading
            };
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(area)
                    .layout(egui::Layout::centered_and_justified(
                        egui::Direction::TopDown,
                    )),
                |ui| {
                    ui.label(
                        egui::RichText::new(self.tr(label))
                            .size(14.0)
                            .color(theme::on_scrim()),
                    );
                },
            );
        }
        ui.advance_cursor_after_rect(area);
        let ready = self.session.project().duration() > Tick::ZERO && self.playback.can_play();
        ui.horizontal(|ui| {
            if self.icon_only(
                ui,
                "transport.back",
                Icon::SkipBack,
                self.tr(Text::Back),
                Some(self.tr(Text::ShortcutBack)),
                None,
                true,
            ) {
                self.action("transport.back");
            }
            let (play_icon, play_text, play_key) = if self.playback.playing() {
                (Icon::Pause, Text::Pause, Text::ShortcutPause)
            } else {
                (Icon::Play, Text::Play, Text::ShortcutPlay)
            };
            if self.icon_only(
                ui,
                "transport.play",
                play_icon,
                self.tr(play_text),
                Some(self.tr(play_key)),
                None,
                ready,
            ) {
                self.action("transport.play");
            }
            if self.icon_only(
                ui,
                "transport.forward",
                Icon::SkipForward,
                self.tr(Text::Forward),
                Some(self.tr(Text::ShortcutForward)),
                None,
                true,
            ) {
                self.action("transport.forward");
            }
        });
        let at = self.playback.position().unwrap_or(Tick::ZERO);
        ui.label(catalog::format(
            self.preferences.locale,
            Text::TimePosition,
            &[
                ("current", &display_time(at)),
                ("duration", &display_time(self.session.project().duration())),
            ],
        ));
        if !self.playback.device_available() {
            ui.label(self.tr(Text::Offline));
        }
        if self.error.is_some() || self.playback.error().is_some() {
            ui.label(self.tr(Text::Unavailable));
            if self.icon_only(
                ui,
                "preview.retry",
                Icon::ArrowClockwise,
                self.tr(Text::Retry),
                None,
                None,
                true,
            ) {
                self.error = None;
                self.action("preview.retry");
            }
        }
    }
    /// Non-modal import overlay over the media panel. Import controls are
    /// already disabled while the batch runs; cancel remains available.
    fn import_overlay(&mut self, ctx: &egui::Context) {
        if !self.importing() {
            return;
        }
        let rect = self
            .panels
            .get("media")
            .copied()
            .unwrap_or_else(|| Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(320.0, 200.0)));
        let percent = self
            .import_completed
            .saturating_mul(100)
            .checked_div(self.import_total)
            .unwrap_or(0)
            .min(100);
        egui::Area::new(Id::unique("import.overlay"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min + Vec2::new(12.0, 12.0))
            .show(ctx, |ui| {
                ui.set_max_width((rect.width() - 24.0).clamp(220.0, 360.0));
                egui::Frame::new()
                    .fill(self.panel_fill())
                    .stroke(egui::Stroke::new(1.0, self.border()))
                    .corner_radius(12)
                    .inner_margin(16)
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(self.tr(Text::Importing))
                                .size(16.0)
                                .strong(),
                        );
                        ui.add_space(10.0);
                        let (bar, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 2.0),
                            egui::Sense::hover(),
                        );
                        ui.painter().rect_filled(bar, 0.0, self.control_fill());
                        let filled = Rect::from_min_size(
                            bar.min,
                            Vec2::new(bar.width() * percent as f32 / 100.0, bar.height()),
                        );
                        ui.painter().rect_filled(filled, 0.0, self.accent());
                        ui.label(catalog::format(
                            self.preferences.locale,
                            Text::ProcessingPercent,
                            &[
                                ("status", self.tr(Text::Processing)),
                                (
                                    "percent",
                                    &catalog::number(self.preferences.locale, percent as u64),
                                ),
                            ],
                        ));
                        ui.label(
                            egui::RichText::new(catalog::format(
                                self.preferences.locale,
                                Text::ImportProgress,
                                &[
                                    (
                                        "done",
                                        &catalog::number(
                                            self.preferences.locale,
                                            self.import_completed as u64,
                                        ),
                                    ),
                                    (
                                        "total",
                                        &catalog::number(
                                            self.preferences.locale,
                                            self.import_total as u64,
                                        ),
                                    ),
                                ],
                            ))
                            .small()
                            .color(self.secondary()),
                        );
                        if self.button(ui, "import.cancel", Icon::X, Text::Cancel, true) {
                            self.action("import.cancel");
                        }
                    });
            });
    }
    fn drop_overlay(&self, ctx: &egui::Context, hovering: bool) {
        if !hovering || self.importing() {
            return;
        }
        let screen = ctx.content_rect();
        egui::Area::new(Id::unique("import.drop"))
            .order(egui::Order::Tooltip)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.set_width(screen.width());
                ui.set_height(screen.height());
                ui.painter().rect_filled(screen, 0.0, theme::scrim());
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(screen.shrink(48.0)).layout(
                        egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    ),
                    |ui| {
                        ui.label(
                            egui::RichText::new(self.tr(Text::DropMedia))
                                .size(18.0)
                                .color(theme::on_scrim()),
                        );
                    },
                );
            });
    }
    fn toast(&self, ctx: &egui::Context) {
        let Some(toast) = &self.toast else {
            return;
        };
        let text = toast.text.clone();
        let detail = toast.detail.clone();
        egui::Area::new(Id::unique("import.toast"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, Vec2::new(-16.0, -16.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.panel_fill())
                    .stroke(egui::Stroke::new(1.0, self.border()))
                    .corner_radius(12)
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.set_max_width(320.0);
                        let response = ui.label(egui::RichText::new(text).size(14.0).strong());
                        response.ctx.accesskit_node_builder(response.id, |node| {
                            node.set_author_id("import.toast");
                        });
                        if let Some(detail) = detail {
                            ui.label(egui::RichText::new(detail).small().color(self.secondary()));
                        }
                    });
            });
    }
    fn timeline(&mut self, ui: &mut egui::Ui) {
        let layout = TimelineLayout::new(ui.max_rect());
        self.timeline_viewport = layout.body.width().max(1.0);
        let min = self.min_zoom(self.timeline_viewport);
        if self.zoom < min {
            self.apply_zoom(min);
        }
        self.timeline_toolbar(ui, &layout);
        self.timeline_scroll_input(ui, &layout);
        let (rows, items, duration, bookmarks) = self.timeline_views(&layout);
        self.paint_ruler(ui, &layout, bookmarks.as_slice());
        self.paint_tracks(ui, &layout, &rows, &items);
        self.advance_drag(ui, &layout, &rows);
        self.paint_playhead(ui, &layout);
        self.timeline_playhead_interaction(ui, &layout);
        self.timeline_scrollbars(ui, &layout, duration, &rows);
        if let Err(error) = self.remember_view() {
            self.error = Some(error.to_string());
        }
        if items.is_empty() && !rows.is_empty() {
            let empty = self.tr(Text::EmptyTimeline);
            ui.painter().text(
                layout.body.center(),
                Align2::CENTER_CENTER,
                empty,
                FontId::proportional(14.0),
                self.secondary(),
            );
        }
    }
    fn scenes_sheet(&mut self, ctx: &egui::Context) {
        if !self.scenes_open {
            return;
        }
        let scenes: Vec<_> = self
            .session
            .project()
            .data()
            .scenes
            .iter()
            .map(|s| (s.id, s.name.as_str().to_owned(), s.is_main))
            .collect();
        let active = self.session.project().data().current_scene_id;
        let sheet_id = Id::unique("scenes.sheet");
        let sheet = egui::Modal::new(sheet_id)
            .area(egui::Modal::default_area(sheet_id).anchor(egui::Align2::RIGHT_TOP, Vec2::ZERO))
            .show(ctx, |ui| {
                ui.set_width(320.0);
                ui.set_min_height((ctx.content_rect().height() - 16.0).max(200.0));
                ui.heading(self.tr(if self.scene_select {
                    Text::SelectScenes
                } else {
                    Text::Scenes
                }));
                ui.label(self.tr(if self.scene_select {
                    Text::SelectScenesHint
                } else {
                    Text::SwitchScenesHint
                }));
                ui.horizontal(|ui| {
                    let response = ui
                        .add(
                            egui::Button::new(icons::tinted(
                                Icon::Plus,
                                BUTTON_ICON,
                                self.text_color(),
                            ))
                            .min_size(Vec2::splat(28.0)),
                        )
                        .on_hover_text(self.tr(Text::NewScene));
                    self.remember_with_icon("scene.create", Some(Icon::Plus.name()), &response);
                    if response.clicked() {
                        self.action("scene.create");
                    }
                    let response = ui.button(self.tr(if self.scene_select {
                        Text::CancelSelection
                    } else {
                        Text::Select
                    }));
                    self.remember("scene.select", &response);
                    if response.clicked() {
                        self.action("scene.select");
                    }
                    let response = ui.button(self.tr(Text::CloseScenes));
                    self.remember("scene.close", &response);
                    if response.clicked() {
                        self.action("scene.close");
                    }
                });
                let editing = self.scene_rename.as_ref().map(|(id, _)| *id);
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        for (id, name, main) in scenes {
                            if editing == Some(id) {
                                let mut text = self
                                    .scene_rename
                                    .as_ref()
                                    .map(|(_, text)| text.clone())
                                    .unwrap_or_default();
                                let response = ui.add(
                                    egui::TextEdit::singleline(&mut text)
                                        .id_salt("scene.rename.input")
                                        .desired_width(296.0)
                                        .min_size(Vec2::new(296.0, 26.0)),
                                );
                                self.remember("scene.rename.input", &response);
                                if self.scene_rename_focus {
                                    response.request_focus();
                                    self.scene_rename_focus = false;
                                }
                                let cancel = !self.scene_confirm
                                    && ui.input(|i| i.key_pressed(egui::Key::Escape));
                                let commit = !self.scene_confirm
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                if commit && !cancel {
                                    match self.session.execute(
                                        "scene.rename",
                                        serde_json::json!({"scene":id,"name":text}),
                                    ) {
                                        Ok(_) => self.scene_rename = None,
                                        Err(error) => {
                                            self.error = Some(error.to_string());
                                            self.scene_rename = Some((id, text));
                                        }
                                    }
                                } else if cancel {
                                    self.scene_rename = None;
                                } else {
                                    self.scene_rename = Some((id, text));
                                }
                                continue;
                            }
                            let selected = if self.scene_select {
                                self.scene_selected.contains(&id)
                            } else {
                                active == id
                            };
                            let label = if main {
                                format!("{name} ({})", self.tr(Text::MainScene))
                            } else {
                                name
                            };
                            let response = ui.add_enabled(
                                !self.scene_select || !main,
                                egui::Button::new(egui::RichText::new(label).color(if selected {
                                    theme::on_accent(self.preferences.dark)
                                } else {
                                    self.text_color()
                                }))
                                .truncate()
                                .selected(selected)
                                .min_size(Vec2::new(296.0, 40.0)),
                            );
                            let rename_id = format!("scene.rename.{}", id.0);
                            let mut request_rename = false;
                            let mut menu_item = None;
                            response.context_menu(|ui| {
                                let item = ui.add(egui::Button::image_and_text(
                                    icons::tinted(
                                        Icon::PencilSimple,
                                        BUTTON_ICON,
                                        self.text_color(),
                                    ),
                                    self.tr(Text::RenameScene),
                                ));
                                if item.clicked() {
                                    request_rename = true;
                                    ui.close();
                                }
                                menu_item = Some(item);
                            });
                            if let Some(item) = menu_item {
                                self.remember_with_icon(
                                    &rename_id,
                                    Some(Icon::PencilSimple.name()),
                                    &item,
                                );
                            }
                            if request_rename {
                                self.action(&rename_id);
                            }
                            let row = format!("scene.row.{}", id.0);
                            self.remember(&row, &response);
                            if response.clicked() {
                                self.action(&row);
                            }
                        }
                    });
                if self.scene_select {
                    let response = ui.add_enabled(
                        !self.scene_selected.is_empty(),
                        egui::Button::image_and_text(
                            icons::tinted(Icon::Trash, BUTTON_ICON, self.text_color()),
                            format!("{} ({})", self.tr(Text::Delete), self.scene_selected.len()),
                        ),
                    );
                    self.remember("scene.delete", &response);
                    if response.clicked() {
                        self.action("scene.delete");
                    }
                }
            });
        if sheet.backdrop_response.clicked() && !self.scene_confirm && self.scene_rename.is_none() {
            self.action("scene.close");
        }
        if self.scene_confirm {
            egui::Modal::new(Id::unique("scene.delete_dialog")).show(ctx, |ui| {
                ui.set_max_width(360.0);
                ui.heading(format!(
                    "{} {}",
                    self.tr(Text::Delete),
                    self.tr(Text::Scenes)
                ));
                ui.label(self.tr(Text::DeleteScenesPrompt));
                ui.horizontal(|ui| {
                    let response =
                        ui.add_sized([90.0, 34.0], egui::Button::new(self.tr(Text::Cancel)));
                    self.remember("scene.cancel_delete", &response);
                    if response.clicked() {
                        self.action("scene.cancel_delete");
                    }
                    let response =
                        ui.add_sized([90.0, 34.0], egui::Button::new(self.tr(Text::Delete)));
                    self.remember("scene.confirm_delete", &response);
                    if response.clicked() {
                        self.action("scene.confirm_delete");
                    }
                });
            });
        }
    }
    fn media_delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(ids) = self.media_delete_confirm.clone() else {
            return;
        };
        egui::Modal::new(Id::unique("media.delete_dialog")).show(ctx, |ui| {
            ui.set_max_width(380.0);
            ui.heading(format!(
                "{} {}",
                self.tr(Text::Delete),
                self.tr(Text::Media)
            ));
            ui.label(catalog::format_count(
                self.preferences.locale,
                ids.len() as u64,
                Text::DeleteMediaPromptOne,
                Text::DeleteMediaPromptOther,
            ));
            ui.horizontal(|ui| {
                let response = ui.add_sized([90.0, 34.0], egui::Button::new(self.tr(Text::Cancel)));
                self.remember("media.cancel_delete", &response);
                if response.clicked() {
                    self.action("media.cancel_delete");
                }
                let response = ui.add_sized([90.0, 34.0], egui::Button::new(self.tr(Text::Delete)));
                self.remember("media.confirm_delete", &response);
                if response.clicked() {
                    self.action("media.confirm_delete");
                }
            });
        });
    }
    fn timeline_toolbar(&mut self, ui: &mut egui::Ui, layout: &TimelineLayout) {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("timeline.toolbar")
                .max_rect(layout.toolbar.shrink2(Vec2::new(4.0, 6.0))),
        );
        child.set_clip_rect(layout.toolbar);
        child.horizontal_centered(|ui| {
            ui.label(
                egui::RichText::new(self.tr(Text::Timeline))
                    .size(16.0)
                    .strong(),
            );
            ui.spacing_mut().item_spacing.x = TOOLBAR_SPACING;
            let can_undo = self.session.can_undo() || self.preference_history.can_undo();
            let can_redo = self.session.can_redo() || self.preference_history.can_redo();
            if self.icon_only(
                ui,
                "edit.undo",
                Icon::ArrowCounterClockwise,
                self.tr(Text::Undo),
                Some(self.tr(Text::ShortcutUndo)),
                None,
                can_undo,
            ) {
                self.action("edit.undo");
            }
            if self.icon_only(
                ui,
                "edit.redo",
                Icon::ArrowClockwise,
                self.tr(Text::Redo),
                Some(self.tr(Text::ShortcutRedo)),
                None,
                can_redo,
            ) {
                self.action("edit.redo");
            }
            let has_items = self
                .session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .any(|track| !track.items.is_empty());
            let has_selection = !self.selection.is_empty();
            if self.icon_only(
                ui,
                "timeline.split",
                Icon::Scissors,
                self.tr(Text::Split),
                Some(self.tr(Text::ShortcutSplit)),
                None,
                has_items,
            ) {
                self.action("timeline.split");
            }
            if self.icon_only(
                ui,
                "timeline.delete",
                Icon::Trash,
                self.tr(Text::Delete),
                Some(self.tr(Text::ShortcutDelete)),
                None,
                has_selection,
            ) {
                self.action("timeline.delete");
            }
            if self.icon_only(
                ui,
                "timeline.add_track",
                Icon::Plus,
                self.tr(Text::AddTrack),
                None,
                None,
                true,
            ) {
                self.action("timeline.add_track");
            }
            if self.icon_only(
                ui,
                "timeline.snap",
                Icon::ArrowsInLineHorizontal,
                self.tr(Text::Snap),
                Some(self.tr(Text::ShortcutSnap)),
                Some(self.snapping),
                true,
            ) {
                self.action("timeline.snap");
            }
            let magnet = self.session.project().data().scene().timeline.magnet;
            if self.icon_only(
                ui,
                "timeline.magnet",
                Icon::Magnet,
                self.tr(Text::Magnet),
                None,
                Some(magnet),
                true,
            ) {
                self.track_command("track.set_magnet", serde_json::json!({"enabled":!magnet}));
            }
            if self.icon_only(
                ui,
                "timeline.ripple",
                Icon::Waveform,
                self.tr(Text::Ripple),
                Some(self.tr(Text::ShortcutRippleDelete)),
                Some(self.ripple),
                true,
            ) {
                self.action("timeline.ripple");
            }
            if self.icon_only(
                ui,
                "timeline.zoom_out",
                Icon::MagnifyingGlassMinus,
                self.tr(Text::ZoomOut),
                None,
                None,
                true,
            ) {
                self.action("timeline.zoom_out");
            }
            let min = self.min_zoom(self.timeline_viewport);
            let mut slider = zoom_to_slider(self.zoom, min);
            ui.spacing_mut().slider_width = 96.0;
            let response = ui.add(
                egui::Slider::new(&mut slider, 0.0..=1.0)
                    .show_value(false)
                    .trailing_fill(true),
            );
            self.remember("timeline.zoom_slider", &response);
            if response.changed() {
                self.apply_zoom(slider_to_zoom(slider, min));
            }
            response.on_hover_text(self.tr(Text::ZoomLevel));
            if self.icon_only(
                ui,
                "timeline.zoom_in",
                Icon::MagnifyingGlassPlus,
                self.tr(Text::ZoomIn),
                None,
                None,
                true,
            ) {
                self.action("timeline.zoom_in");
            }
        });
        let bounds = Rect::from_center_size(layout.toolbar.center(), Vec2::new(178.0, 28.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(bounds), |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let name = self
                    .session
                    .project()
                    .data()
                    .scene()
                    .name
                    .as_str()
                    .to_owned();
                let response = ui
                    .add_sized([140.0, 26.0], egui::Button::new(name).truncate())
                    .on_hover_text(self.tr(Text::Scenes));
                self.remember("scene.open", &response);
                if response.clicked() {
                    self.action("scene.open");
                }
                let response = ui
                    .add(egui::Button::image(icons::tinted(
                        Icon::FilmStrip,
                        BUTTON_ICON,
                        self.text_color(),
                    )))
                    .on_hover_text(self.tr(Text::Scenes));
                self.remember("scene.open_list", &response);
                if response.clicked() {
                    self.action("scene.open");
                }
            });
        });
    }
    fn timeline_scroll_input(&mut self, ui: &egui::Ui, layout: &TimelineLayout) {
        if self.drag.is_some() {
            return;
        }
        let hovered = ui.rect_contains_pointer(layout.body)
            || ui.rect_contains_pointer(layout.ruler)
            || ui.rect_contains_pointer(layout.labels);
        if !hovered {
            return;
        }
        let (scroll, zoom, modifiers) = ui.ctx().input(|input| {
            (
                input.smooth_scroll_delta(),
                input.zoom_delta(),
                input.modifiers,
            )
        });
        let (_, max_scroll) = self.scroll_bounds(layout, self.session.project().duration());
        if (zoom - 1.0).abs() > 0.01 {
            self.apply_zoom(self.zoom * zoom);
        } else if modifiers.shift || scroll.x.abs() > scroll.y.abs() {
            let raw = if modifiers.shift && scroll.x.abs() <= f32::EPSILON {
                scroll.y
            } else if scroll.x.abs() > scroll.y.abs() {
                scroll.x
            } else {
                scroll.y
            };
            let delta = raw.clamp(-HORIZONTAL_WHEEL_STEP, HORIZONTAL_WHEEL_STEP);
            self.scroll = (self.scroll - delta / self.zoom).clamp(0.0, max_scroll);
        } else {
            self.vertical_scroll = (self.vertical_scroll - scroll.y).max(0.0);
        }
    }
    fn timeline_views(
        &mut self,
        layout: &TimelineLayout,
    ) -> (Vec<TrackRow>, Vec<ItemView>, Tick, Vec<Tick>) {
        let (rows, raw_items, duration, bookmarks, content_height) = {
            let data = self.session.project().data();
            let mut rows = Vec::new();
            let mut raw_items = Vec::new();
            let mut offset = 0.0;
            let mut overlay_index = 0usize;
            let mut audio_index = 0usize;
            for (row_index, track) in data.scene().timeline.iter().rev().enumerate() {
                let height = track_height(track.kind);
                let main = track.id == data.scene().timeline.main.id;
                let audio = track.kind == TrackKind::Audio;
                let display_index = if main {
                    0
                } else if audio {
                    let index = audio_index;
                    audio_index += 1;
                    index
                } else {
                    let index = overlay_index;
                    overlay_index += 1;
                    index
                };
                rows.push(TrackRow {
                    id: track.id,
                    main,
                    audio,
                    display_index,
                    muted: track.muted,
                    hidden: track.hidden,
                    locked: track.locked,
                    empty: track.items.is_empty(),
                    kind: track.kind,
                    name: track.name.as_str().to_owned(),
                    top: offset,
                    height,
                });
                for item in &track.items {
                    let Ok(end) = item.end() else {
                        continue;
                    };
                    let (label, overlay) = item_label(
                        &item.content,
                        self.preferences.locale,
                        self.preferences.dark,
                    );
                    raw_items.push((
                        row_index,
                        item.id,
                        track.kind,
                        item.start,
                        end,
                        track.locked,
                        label,
                        overlay,
                        item.source_asset,
                    ));
                }
                offset += height + TRACK_GAP;
            }
            (
                rows,
                raw_items,
                self.session.project().duration(),
                data.scene()
                    .bookmarks
                    .iter()
                    .map(|bookmark| bookmark.start)
                    .collect(),
                (offset - TRACK_GAP).max(0.0),
            )
        };
        self.vertical_scroll = self
            .vertical_scroll
            .clamp(0.0, (content_height - layout.body.height()).max(0.0));
        let mut items = Vec::new();
        for (row_index, id, kind, start, end, locked, label, overlay, source_asset) in raw_items {
            let (top, height) = rows
                .get(row_index)
                .map_or((layout.body.top(), 25.0), |row| {
                    (
                        layout.body.top() + row.top - self.vertical_scroll,
                        row.height,
                    )
                });
            items.push(ItemView {
                id,
                kind,
                start,
                end,
                top,
                height,
                locked,
                label,
                overlay,
                source_asset,
            });
        }
        let min = self.min_zoom(layout.body.width());
        let max_scroll =
            max_scroll_seconds(duration.seconds(), layout.body.width(), self.zoom, min);
        self.scroll = self.scroll.clamp(0.0, max_scroll);
        (rows, items, duration, bookmarks)
    }
    fn paint_ruler(&mut self, ui: &egui::Ui, layout: &TimelineLayout, bookmarks: &[Tick]) {
        let painter = ui.painter().with_clip_rect(layout.ruler);
        painter.rect_filled(layout.ruler, 0.0, self.control_fill());
        painter.rect_filled(layout.corner, 0.0, self.control_fill());
        let rate = self.session.project().data().canvas.fps;
        let config = ruler_config(self.zoom, rate);
        let start = f64::from(self.scroll);
        let end = start + f64::from(layout.ruler.width() / self.zoom);
        let first = (start / config.tick_seconds).floor() as i64;
        let last = (end / config.tick_seconds).ceil() as i64;
        let mut index = first;
        let mut guard = 0;
        while index <= last && guard < 4096 {
            let seconds = index as f64 * config.tick_seconds;
            let labelled = index % config.label_multiple == 0;
            let x = if labelled {
                snapped_seconds(seconds, rate)
            } else {
                seconds
            };
            let x = layout.ruler.left() + ((x - start) * f64::from(self.zoom)) as f32;
            if x >= layout.ruler.left() - 1.0 && x <= layout.ruler.right() + 1.0 {
                let height = if labelled { 8.0 } else { 4.0 };
                painter.line_segment(
                    [
                        egui::pos2(x, layout.ruler.bottom() - height),
                        egui::pos2(x, layout.ruler.bottom()),
                    ],
                    egui::Stroke::new(1.0, self.border()),
                );
                if labelled {
                    painter.text(
                        egui::pos2(x + 4.0, layout.ruler.top() + 2.0),
                        Align2::LEFT_TOP,
                        ruler_label(seconds, rate),
                        FontId::proportional(10.0),
                        self.secondary(),
                    );
                }
            }
            index += 1;
            guard += 1;
        }
        for bookmark in bookmarks {
            let x = self.x_at(*bookmark, layout.ruler.left());
            if x >= layout.ruler.left() && x <= layout.ruler.right() {
                painter.rect_filled(
                    Rect::from_min_size(
                        egui::pos2(x - 2.0, layout.ruler.bottom() - 5.0),
                        Vec2::new(4.0, 5.0),
                    ),
                    1.0,
                    self.accent(),
                );
            }
        }
        let response = ui.interact(
            layout.ruler,
            Id::unique("timeline.ruler"),
            egui::Sense::click_and_drag(),
        );
        self.remember("timeline.ruler", &response);
        if (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let at = self.frame_snap(self.tick_at(pointer.x, layout.ruler.left()));
            self.seek_to(at);
        }
    }
    fn paint_tracks(
        &mut self,
        ui: &mut egui::Ui,
        layout: &TimelineLayout,
        rows: &[TrackRow],
        items: &[ItemView],
    ) {
        let mut header_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("track.headers")
                .max_rect(layout.labels),
        );
        header_ui.set_clip_rect(layout.labels);
        let body_painter = ui.painter().with_clip_rect(layout.body);
        let label_painter = ui.painter().with_clip_rect(layout.labels);
        label_painter.rect_filled(layout.labels, 0.0, self.panel_fill());
        body_painter.rect_filled(layout.body, 0.0, self.panel_fill());
        for (index, row) in rows.iter().enumerate() {
            let top = layout.body.top() + row.top - self.vertical_scroll;
            let label_rect = Rect::from_min_size(
                egui::pos2(layout.labels.left(), top),
                Vec2::new(layout.labels.width(), row.height),
            );
            let row_rect = Rect::from_min_size(
                egui::pos2(layout.body.left(), top),
                Vec2::new(layout.body.width(), row.height),
            );
            if !row_rect.intersects(layout.body) {
                continue;
            }
            let shade = if index % 2 == 0 {
                self.panel_fill()
            } else {
                self.control_fill()
            };
            label_painter.rect_filled(label_rect, 0.0, shade);
            body_painter.rect_filled(row_rect, 0.0, shade);
            let header = header_ui.interact(
                label_rect.intersect(layout.labels),
                Id::unique(("track.header", row.id.0)),
                egui::Sense::click_and_drag(),
            );
            header.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &row.name));
            let type_icon = track_icon(row.kind);
            self.remember_with_icon(
                format!("track.{}.header", row.id.0),
                Some(type_icon.name()),
                &header,
            );
            if row.locked {
                label_painter.rect_filled(
                    label_rect,
                    0.0,
                    theme::warning(self.preferences.dark).gamma_multiply(0.08),
                );
            }
            if self.track_drag == Some(row.id) {
                label_painter.rect_stroke(
                    label_rect,
                    0.0,
                    egui::Stroke::new(1.0, self.accent()),
                    egui::StrokeKind::Inside,
                );
            }
            let name_on_top = row.kind.can_hide() && row.kind.can_mute();
            let name_rect = Rect::from_min_max(
                egui::pos2(label_rect.left() + 4.0, label_rect.top() + 2.0),
                egui::pos2(
                    (label_rect.right() - if name_on_top { 4.0 } else { 58.0 })
                        .max(label_rect.left() + 4.0),
                    if name_on_top {
                        label_rect.top() + 20.0
                    } else {
                        label_rect.bottom() - 2.0
                    },
                ),
            );
            let renaming = self
                .track_rename
                .as_ref()
                .is_some_and(|(track, _)| *track == row.id);
            if renaming {
                let mut name = self
                    .track_rename
                    .take()
                    .map_or_else(String::new, |(_, name)| name);
                let response = header_ui
                    .scope_builder(
                        egui::UiBuilder::new().id_salt(("track.rename.editor", row.id.0)),
                        |ui| {
                            ui.spacing_mut().interact_size.y = name_rect.height();
                            ui.put(
                                name_rect,
                                egui::TextEdit::singleline(&mut name)
                                    .id_salt(("track.rename.input", row.id.0))
                                    .font(FontId::proportional(12.0))
                                    .margin(Vec2::splat(1.0))
                                    .desired_width(name_rect.width()),
                            )
                        },
                    )
                    .inner;
                self.remember(format!("track.{}.rename", row.id.0), &response);
                if self.track_rename_focus {
                    response.request_focus();
                    self.track_rename_focus = false;
                }
                let cancel = header_ui.input(|input| input.key_pressed(egui::Key::Escape));
                let commit = header_ui.input(|input| input.key_pressed(egui::Key::Enter));
                if commit && !cancel {
                    self.track_command(
                        "track.rename",
                        serde_json::json!({"track":row.id,"name":name}),
                    );
                } else if !cancel {
                    self.track_rename = Some((row.id, name));
                }
            } else {
                label_painter.with_clip_rect(name_rect).text(
                    egui::pos2(name_rect.left(), name_rect.center().y),
                    Align2::LEFT_CENTER,
                    &row.name,
                    FontId::proportional(if row.height >= 40.0 { 12.0 } else { 10.0 }),
                    if row.locked {
                        theme::warning(self.preferences.dark)
                    } else {
                        self.text_color()
                    },
                );
            }
            let icon_rect = Rect::from_center_size(
                egui::pos2(label_rect.right() - 14.0, label_rect.center().y),
                Vec2::splat(16.0),
            );
            icons::tinted(type_icon, 16.0, self.secondary()).paint_at(&header_ui, icon_rect);
            let mut x = label_rect.right() - 54.0;
            if row.kind.can_hide() {
                if self.track_toggle(
                    &mut header_ui,
                    layout.labels,
                    row,
                    x,
                    "hidden",
                    Text::HideTrack,
                    row.hidden,
                    if row.hidden {
                        Icon::EyeSlash
                    } else {
                        Icon::Eye
                    },
                ) {
                    self.track_command(
                        "track.set_hidden",
                        serde_json::json!({"track":row.id,"hidden":!row.hidden}),
                    );
                }
                x -= 28.0;
            }
            if row.kind.can_mute()
                && self.track_toggle(
                    &mut header_ui,
                    layout.labels,
                    row,
                    x,
                    "muted",
                    Text::MuteTrack,
                    row.muted,
                    if row.muted {
                        Icon::SpeakerSlash
                    } else {
                        Icon::SpeakerHigh
                    },
                )
            {
                self.track_command(
                    "track.set_muted",
                    serde_json::json!({"track":row.id,"muted":!row.muted}),
                );
            }
            if row.main {
                let enabled = self.session.project().data().scene().timeline.magnet;
                // A separate control fits below the main header's output toggles.
                let mut magnet_row = row.clone();
                magnet_row.top += 18.0;
                if self.track_toggle(
                    &mut header_ui,
                    layout.labels,
                    &magnet_row,
                    label_rect.left() + 4.0,
                    "magnet",
                    Text::Magnet,
                    enabled,
                    Icon::Magnet,
                ) {
                    self.track_command("track.set_magnet", serde_json::json!({"enabled":!enabled}));
                }
            }
            header.context_menu(|ui| {
                let rename = ui.add(egui::Button::image_and_text(
                    icons::tinted(Icon::PencilSimple, 16.0, self.text_color()),
                    self.tr(Text::RenameTrack),
                ));
                self.remember_with_icon(
                    format!("track.{}.rename_action", row.id.0),
                    Some(Icon::PencilSimple.name()),
                    &rename,
                );
                if rename.clicked() {
                    self.track_rename = Some((row.id, row.name.clone()));
                    self.track_rename_focus = true;
                    ui.close();
                }
                let lock_icon = if row.locked {
                    Icon::LockOpen
                } else {
                    Icon::Lock
                };
                let lock_text = if row.locked {
                    self.tr(Text::UnlockTrack)
                } else {
                    self.tr(Text::LockTrack)
                };
                let lock = ui.add(egui::Button::image_and_text(
                    icons::tinted(lock_icon, 16.0, self.text_color()),
                    lock_text,
                ));
                self.remember_with_icon(
                    format!("track.{}.lock", row.id.0),
                    Some(lock_icon.name()),
                    &lock,
                );
                if lock.clicked() {
                    self.track_command(
                        "track.set_locked",
                        serde_json::json!({"track":row.id,"locked":!row.locked}),
                    );
                    ui.close();
                }
                if !row.main {
                    let response = ui.add_enabled(
                        !row.locked,
                        egui::Button::image_and_text(
                            icons::tinted(Icon::Trash, 16.0, self.text_color()),
                            self.tr(Text::DeleteTrack),
                        ),
                    );
                    let response = response.on_disabled_hover_text(self.tr(Text::TrackLocked));
                    self.remember_with_icon(
                        format!("track.{}.delete", row.id.0),
                        Some(Icon::Trash.name()),
                        &response,
                    );
                    if response.clicked() {
                        self.track_command("track.remove", serde_json::json!({"track":row.id}));
                        ui.close();
                    }
                }
            });
            if header.drag_started() {
                self.track_drag = Some(row.id);
            }
            if header.drag_stopped() && self.track_drag == Some(row.id) {
                self.track_drag = None;
                let drop = ui.input(|input| input.pointer.interact_pos());
                if !row.main
                    && let Some(drop) = drop
                    && let Some(target) = rows.iter().find(|candidate| {
                        !candidate.main
                            && candidate.audio == row.audio
                            && drop.y >= layout.body.top() + candidate.top - self.vertical_scroll
                            && drop.y
                                <= layout.body.top() + candidate.top - self.vertical_scroll
                                    + candidate.height
                    })
                    && target.id != row.id
                {
                    self.track_command(
                        "track.reorder",
                        serde_json::json!({"track":row.id,"index":target.display_index}),
                    );
                }
            }
            if row.empty {
                let r = row_rect.shrink(3.0);
                let points = [
                    r.left_top(),
                    r.right_top(),
                    r.right_bottom(),
                    r.left_bottom(),
                    r.left_top(),
                ];
                body_painter.extend(egui::Shape::dashed_line(
                    &points,
                    egui::Stroke::new(1.0, self.border()),
                    5.0,
                    4.0,
                ));
            }
        }
        let background = ui.interact(
            layout.body,
            Id::unique("timeline.marquee"),
            egui::Sense::click_and_drag(),
        );
        self.remember("timeline.marquee", &background);
        let modifiers = ui.ctx().input(|input| input.modifiers);
        for item in items {
            self.paint_item(ui, layout, item, modifiers);
        }
        let item_rects: Vec<_> = items
            .iter()
            .map(|item| {
                (
                    item.id,
                    Rect::from_min_max(
                        egui::pos2(self.x_at(item.start, layout.body.left()), item.top + 2.0),
                        egui::pos2(
                            self.x_at(item.end, layout.body.left()),
                            item.top + item.height - 2.0,
                        ),
                    )
                    .intersect(layout.body),
                )
            })
            .filter(|(_, rect)| rect.is_positive())
            .collect();
        self.effect_drop_targets = item_rects.clone();
        if self.effect_drag.is_some() {
            let accent = self.accent();
            for (id, rect) in &item_rects {
                if self.effect_refusal(*id).is_none() {
                    body_painter.rect_stroke(
                        *rect,
                        2.0,
                        egui::Stroke::new(2.0, accent),
                        egui::StrokeKind::Inside,
                    );
                }
            }
        }
        if background.drag_started()
            && self.drag.is_none()
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
            && !item_rects.iter().any(|(_, r)| r.contains(origin))
        {
            self.marquee = Some((
                origin,
                self.selection.clone(),
                modifiers.command || modifiers.ctrl || modifiers.shift,
            ));
        }
        if let Some((origin, before, additive)) = &self.marquee
            && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
        {
            let rect = Rect::from_two_pos(*origin, pointer).intersect(layout.body);
            let mut selected =
                crate::selection::intersections(rect.min, rect.max, item_rects.iter().copied());
            if *additive {
                selected.extend(before);
            }
            self.selection = selected;
            ui.painter().with_clip_rect(layout.body).rect_filled(
                rect,
                0.0,
                self.accent().gamma_multiply(0.15),
            );
            ui.painter().with_clip_rect(layout.body).rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0, self.accent()),
                egui::StrokeKind::Inside,
            );
        }
        if background.drag_stopped() && self.marquee.take().is_some() {
            self.selection_changed();
        }
        if background.clicked() {
            self.selection.clear();
            self.selection_changed();
        }
        if let Some(at) = self.snap_indicator {
            let x = self.x_at(at, layout.body.left());
            if x >= layout.body.left() && x <= layout.body.right() {
                ui.painter().with_clip_rect(layout.body).line_segment(
                    [
                        egui::pos2(x, layout.body.top()),
                        egui::pos2(x, layout.body.bottom()),
                    ],
                    egui::Stroke::new(1.0, self.accent()),
                );
            }
        }
    }
    /// Add a secondary track compatible with the active section: audio beside an
    /// audio track, otherwise the active overlay kind (Video when nothing is
    /// selected). Above the active track, or the overlay section end otherwise.
    fn add_track(&mut self) {
        let locale = self.preferences.locale;
        let (kind, placement, count) = {
            let data = self.session.project().data();
            let scene = data.scene();
            let active = self.selection.iter().find_map(|id| {
                scene
                    .timeline
                    .iter()
                    .find(|track| track.items.iter().any(|item| item.id == *id))
            });
            let kind = active.map_or(TrackKind::Video, |track| match track.kind {
                TrackKind::Audio => TrackKind::Audio,
                TrackKind::Text => TrackKind::Text,
                _ => TrackKind::Video,
            });
            let placement = match active {
                Some(track) if track.id != scene.timeline.main.id => {
                    serde_json::json!({"mode":"above","track":track.id})
                }
                _ => serde_json::json!({"mode":"above","track":scene.timeline.main.id}),
            };
            let count = scene
                .timeline
                .iter()
                .filter(|track| track.kind == kind)
                .count()
                + 1;
            (kind, placement, count)
        };
        let number = catalog::number(locale, count as u64);
        let name = match kind {
            TrackKind::Audio => catalog::format(locale, Text::AudioTrackName, &[("n", &number)]),
            TrackKind::Text => self.tr(Text::TextItem).to_owned(),
            _ => catalog::format(locale, Text::VideoTrackName, &[("n", &number)]),
        };
        self.track_command(
            "timeline.add_track",
            serde_json::json!({"name":name,"kind":kind,"placement":placement}),
        );
    }
    fn track_command(&mut self, command: &str, params: serde_json::Value) {
        let before = self.session.project().clone();
        match self
            .session
            .set_selection(self.selection.clone())
            .and_then(|_| self.session.execute(command, params))
        {
            Ok(_) => {
                self.prune_selection();
                if !Arc::ptr_eq(&before, self.session.project()) {
                    self.sync_preview();
                }
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn track_toggle(
        &mut self,
        ui: &mut egui::Ui,
        clip: Rect,
        row: &TrackRow,
        x: f32,
        suffix: &str,
        label: Text,
        selected: bool,
        icon: Icon,
    ) -> bool {
        let rect = Rect::from_min_size(
            egui::pos2(
                x,
                clip.top() + row.top - self.vertical_scroll + (row.height - 24.0) / 2.0,
            ),
            Vec2::splat(ICON_BUTTON),
        )
        .intersect(clip);
        if !rect.is_positive() {
            return false;
        }
        let response = ui.interact(
            rect,
            Id::unique(("track", row.id.0, suffix)),
            egui::Sense::click(),
        );
        let tooltip = catalog::format(
            self.preferences.locale,
            Text::TrackToggle,
            &[("action", self.tr(label)), ("track", &row.name)],
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::CheckBox, true, selected, tooltip.clone())
        });
        self.remember_with_icon(
            format!("track.{}.{suffix}", row.id.0),
            Some(icon.name()),
            &response,
        );
        if let Some(state) = self
            .widgets
            .get_mut(&format!("track.{}.{suffix}", row.id.0))
        {
            state.tooltip = Some(tooltip.clone());
        }
        let color = if selected {
            self.accent()
        } else {
            self.secondary()
        };
        if response.hovered() || response.has_focus() {
            ui.painter()
                .with_clip_rect(clip)
                .rect_filled(rect, 4.0, self.control_fill());
        }
        icons::tinted(icon, 16.0, color)
            .paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(16.0)));
        response.clone().on_hover_text(tooltip);
        response.clicked()
    }
    fn paint_item(
        &mut self,
        ui: &egui::Ui,
        layout: &TimelineLayout,
        item: &ItemView,
        modifiers: egui::Modifiers,
    ) {
        let x0 = self.x_at(item.start, layout.body.left());
        let x1 = self.x_at(item.end, layout.body.left());
        let full = Rect::from_min_max(
            egui::pos2(x0, item.top + 2.0),
            egui::pos2(x1, item.top + item.height - 2.0),
        );
        let visible = full.intersect(layout.body);
        if !visible.is_positive() {
            return;
        }
        let selected = self.selection.contains(&item.id);
        let response = ui.interact(
            visible,
            Id::unique(("timeline.item", item.id.0)),
            if item.locked {
                egui::Sense::click()
            } else {
                egui::Sense::click_and_drag()
            },
        );
        self.remember(format!("timeline.item.{}", item.id.0), &response);
        let (fill, border, text) =
            item_palette(item.kind, self.preferences.dark, selected, item.overlay);
        let painter = ui.painter().with_clip_rect(visible);
        painter.rect_filled(visible, 0.0, fill);
        painter.rect_stroke(
            visible,
            0.0,
            egui::Stroke::new(if selected { 2.0 } else { 1.0 }, border),
            egui::StrokeKind::Inside,
        );
        match item.kind {
            TrackKind::Video if item.overlay.is_none() => {
                let mut x = visible.left() + 4.0;
                let mut index = 0;
                while x + 10.0 < visible.right() && index < 256 {
                    let shade = if index % 2 == 0 { 12 } else { 6 };
                    painter.rect_filled(
                        Rect::from_min_size(
                            egui::pos2(x, visible.top() + 3.0),
                            Vec2::new(9.0, visible.height() - 6.0),
                        ),
                        0.0,
                        Color32::from_rgba_unmultiplied(text.r(), text.g(), text.b(), shade),
                    );
                    x += 13.0;
                    index += 1;
                }
            }
            TrackKind::Audio => {
                let center = visible.center().y;
                let bars = ((visible.width() / 4.0).floor() as usize).min(256);
                for index in 0..bars {
                    let t = index as f32 / bars.max(1) as f32;
                    let scale = ((t * 47.0 + item.id.0 as f32).sin() * 0.5 + 0.5) * 0.8 + 0.1;
                    let half = (visible.height() * 0.35 * scale).max(1.0);
                    let x = visible.left() + index as f32 * 4.0 + 2.0;
                    painter.line_segment(
                        [egui::pos2(x, center - half), egui::pos2(x, center + half)],
                        egui::Stroke::new(1.0, text),
                    );
                }
            }
            _ => {}
        }
        painter.text(
            egui::pos2(visible.left() + 6.0, visible.center().y),
            Align2::LEFT_CENTER,
            &item.label,
            FontId::proportional(11.0),
            text,
        );
        if !item.locked && visible.width() > 16.0 {
            let width = 8.0_f32.min(visible.width() / 3.0);
            let left = Rect::from_min_max(
                visible.left_top(),
                egui::pos2(visible.left() + width, visible.bottom()),
            );
            let right = Rect::from_min_max(
                egui::pos2(visible.right() - width, visible.top()),
                visible.right_bottom(),
            );
            for (id, rect, mode) in [
                ("trim_start", left, DragMode::TrimStart),
                ("trim_end", right, DragMode::TrimEnd),
            ] {
                let response = ui.interact(
                    rect,
                    Id::unique(("timeline.item.handle", item.id.0, id)),
                    egui::Sense::click_and_drag(),
                );
                self.remember(format!("timeline.item.{}.{id}", item.id.0), &response);
                ui.painter().with_clip_rect(rect).line_segment(
                    [rect.center_top(), rect.center_bottom()],
                    egui::Stroke::new(
                        2.0,
                        if selected || response.hovered() {
                            self.accent()
                        } else {
                            border
                        },
                    ),
                );
                if response.clicked() {
                    self.select_item(item.id, modifiers);
                }
                if response.drag_started() && self.drag.is_none() {
                    self.begin_drag(item.id, mode, ui);
                }
            }
        }
        if response.clicked() {
            self.select_item(item.id, modifiers);
        }
        if !item.locked && self.drag.is_none() && response.drag_started() {
            self.begin_drag(item.id, DragMode::Move, ui);
        }
        if let Some(asset) = item.source_asset {
            response.context_menu(|ui| {
                let reveal = ui.button(self.tr(Text::RevealMedia));
                self.remember(format!("timeline.item.{}.reveal", item.id.0), &reveal);
                if reveal.clicked() {
                    self.action(&format!("media.reveal.{}", asset.0));
                    ui.close();
                }
            });
        }
    }
    fn paint_playhead(&self, ui: &egui::Ui, layout: &TimelineLayout) {
        let at = self.playback.position().unwrap_or(Tick::ZERO);
        let x = self.x_at(at, layout.body.left());
        if x < layout.body.left() - 2.0 || x > layout.body.right() + 2.0 {
            return;
        }
        let painter = ui.painter().with_clip_rect(Rect::from_min_max(
            egui::pos2(layout.ruler.left(), layout.ruler.top()),
            egui::pos2(layout.ruler.right(), layout.body.bottom()),
        ));
        painter.line_segment(
            [
                egui::pos2(x, layout.ruler.bottom()),
                egui::pos2(x, layout.body.bottom()),
            ],
            egui::Stroke::new(2.0, self.accent()),
        );
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(x - 5.0, layout.ruler.bottom() - 8.0),
                egui::pos2(x + 5.0, layout.ruler.bottom() - 8.0),
                egui::pos2(x, layout.ruler.bottom()),
            ],
            self.accent(),
            egui::Stroke::NONE,
        ));
    }
    fn timeline_playhead_interaction(&mut self, ui: &egui::Ui, layout: &TimelineLayout) {
        let at = self.playback.position().unwrap_or(Tick::ZERO);
        let x = self.x_at(at, layout.body.left());
        if x < layout.body.left() - 6.0 || x > layout.body.right() + 6.0 {
            return;
        }
        // Keep the playhead draggable from the ruler so item automation IDs stay
        // clickable when the line crosses a clip.
        let rect = Rect::from_min_max(
            egui::pos2(x - 6.0, layout.ruler.top()),
            egui::pos2(x + 6.0, layout.ruler.bottom()),
        );
        let response = ui.interact(
            rect,
            Id::unique("timeline.playhead"),
            egui::Sense::click_and_drag(),
        );
        self.remember("timeline.playhead", &response);
        if (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let at = self.frame_snap(self.tick_at(pointer.x, layout.body.left()));
            self.seek_to(at);
        }
    }
    fn timeline_scrollbars(
        &mut self,
        ui: &egui::Ui,
        layout: &TimelineLayout,
        duration: Tick,
        rows: &[TrackRow],
    ) {
        let (view, max) = self.scroll_bounds(layout, duration);
        let content = view + max;
        let track = layout.horizontal.shrink2(Vec2::new(0.0, 3.0));
        let thumb_width =
            ((view / content) * track.width()).clamp(12.0_f32.min(track.width()), track.width());
        let fraction = if max > 0.0 { self.scroll / max } else { 0.0 };
        let thumb_x = track.left() + fraction * (track.width() - thumb_width);
        let thumb = Rect::from_min_size(
            egui::pos2(thumb_x, track.top()),
            Vec2::new(thumb_width, track.height()),
        );
        ui.painter()
            .rect_filled(layout.horizontal, 0.0, self.panel_fill());
        ui.painter().rect_filled(track, 6.0, self.control_fill());
        ui.painter().rect_filled(thumb, 6.0, self.border());
        let response = ui.interact(
            layout.horizontal,
            Id::unique("timeline.scroll_h"),
            egui::Sense::click_and_drag(),
        );
        self.remember("timeline.scroll_h", &response);
        if (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
            && max > 0.0
            && track.width() > 0.0
        {
            let fraction = ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0);
            self.scroll = fraction * max;
        }
        let content_height: f32 = rows.iter().map(|row| row.height + TRACK_GAP).sum::<f32>();
        let max_vertical = (content_height - TRACK_GAP - layout.body.height()).max(0.0);
        let track = layout.vertical.shrink2(Vec2::new(3.0, 0.0));
        let thumb_height = if content_height > 0.0 {
            ((layout.body.height() / content_height) * track.height())
                .clamp(12.0_f32.min(track.height()), track.height())
        } else {
            track.height()
        };
        let fraction = if max_vertical > 0.0 {
            self.vertical_scroll / max_vertical
        } else {
            0.0
        };
        let thumb_y = track.top() + fraction * (track.height() - thumb_height);
        ui.painter()
            .rect_filled(layout.vertical, 0.0, self.panel_fill());
        ui.painter().rect_filled(track, 6.0, self.control_fill());
        ui.painter().rect_filled(
            Rect::from_min_size(
                egui::pos2(track.left(), thumb_y),
                Vec2::new(track.width(), thumb_height),
            ),
            6.0,
            self.border(),
        );
        let response = ui.interact(
            layout.vertical,
            Id::unique("timeline.scroll_v"),
            egui::Sense::click_and_drag(),
        );
        self.remember("timeline.scroll_v", &response);
        if (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
            && max_vertical > 0.0
            && track.height() > 0.0
        {
            let fraction = ((pointer.y - track.top()) / track.height()).clamp(0.0, 1.0);
            self.vertical_scroll = fraction * max_vertical;
        }
    }
    fn frame_snap(&self, at: Tick) -> Tick {
        self.session
            .project()
            .data()
            .canvas
            .fps
            .snap_nearest(at)
            .unwrap_or(at)
    }
    fn accent(&self) -> Color32 {
        theme::accent(self.preferences.dark)
    }
    fn panel_fill(&self) -> Color32 {
        theme::panel(self.preferences.dark)
    }
    fn control_fill(&self) -> Color32 {
        theme::field(self.preferences.dark)
    }
    fn border(&self) -> Color32 {
        theme::border(self.preferences.dark)
    }
    fn text_color(&self) -> Color32 {
        theme::text(self.preferences.dark)
    }
    fn secondary(&self) -> Color32 {
        theme::secondary(self.preferences.dark)
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
/// Paint top/bottom fades over a scroll view whose content continues past the edge.
fn scroll_fades<F>(ui: &egui::Ui, output: &egui::scroll_area::ScrollAreaOutput<F>, dark: bool) {
    const STRIPS: usize = 6;
    const FADE_HEIGHT: f32 = 24.0;
    let view = output.inner_rect;
    let offset = output.state.clamped_offset().y;
    let show_top = offset > 0.5;
    let show_bottom = offset + view.height() < output.content_size.y - 0.5;
    if !show_top && !show_bottom {
        return;
    }
    let base = theme::panel(dark);
    let strip = (FADE_HEIGHT / STRIPS as f32).min(view.height() / 2.0);
    for index in 0..STRIPS {
        let alpha = (255.0 * (1.0 - index as f32 / STRIPS as f32)).round() as u8;
        let color = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha);
        if show_top {
            ui.painter().rect_filled(
                Rect::from_min_size(
                    egui::pos2(view.left(), view.top() + index as f32 * strip),
                    Vec2::new(view.width(), strip),
                ),
                0.0,
                color,
            );
        }
        if show_bottom {
            ui.painter().rect_filled(
                Rect::from_min_size(
                    egui::pos2(view.left(), view.bottom() - (index as f32 + 1.0) * strip),
                    Vec2::new(view.width(), strip),
                ),
                0.0,
                color,
            );
        }
    }
}
fn item_patch(field: &str, value: &str) -> Result<ItemParamPatch> {
    Ok(match field {
        "blendMode" => ItemParamPatch::BlendMode(
            serde_json::from_value(serde_json::json!(value))
                .map_err(|_| Error::Invalid("blend mode"))?,
        ),
        "opacity" => ItemParamPatch::Opacity(
            value
                .trim()
                .parse()
                .map_err(|_| Error::Invalid("opacity"))?,
        ),
        "audio_gain" => ItemParamPatch::AudioGain(
            value
                .trim()
                .parse()
                .map_err(|_| Error::Invalid("audio gain"))?,
        ),
        "audio_muted" => {
            let muted = match value.trim() {
                "true" | "1" | "muted" => true,
                "false" | "0" | "unmuted" => false,
                _ => return Err(Error::Invalid("audio mute")),
            };
            ItemParamPatch::AudioMuted(muted)
        }
        "text_content" => ItemParamPatch::TextContent(value.into()),
        "text_font" => {
            let font = value.trim();
            if font.is_empty() || font.len() > 256 {
                return Err(Error::Invalid("font family"));
            }
            ItemParamPatch::TextFont(font.into())
        }
        "text_size" => ItemParamPatch::TextSize(
            value
                .trim()
                .parse()
                .map_err(|_| Error::Invalid("text size"))?,
        ),
        "text_color" => ItemParamPatch::TextColor(parse_color(value)?),
        "text_background" => {
            let value = value.trim();
            ItemParamPatch::TextBackground(if value.eq_ignore_ascii_case("none") {
                None
            } else {
                Some(project::TextBackground {
                    color: parse_color(value)?,
                    ..Default::default()
                })
            })
        }
        "text_background_color" => ItemParamPatch::TextBackgroundColor(parse_color(value)?),
        "text_position_x" => ItemParamPatch::TextPositionX(
            parse_percent(value).map_err(|_| Error::Invalid("horizontal position"))?,
        ),
        "text_position_y" => ItemParamPatch::TextPositionY(
            parse_percent(value).map_err(|_| Error::Invalid("vertical position"))?,
        ),
        _ => return Err(Error::Invalid("shared field")),
    })
}
fn asset_kind(asset: &SourceAsset) -> std::result::Result<TrackKind, engine::EditError> {
    if asset
        .probe()
        .streams
        .iter()
        .any(|stream| stream.kind == StreamKind::Video)
    {
        Ok(TrackKind::Video)
    } else if asset
        .probe()
        .streams
        .iter()
        .any(|stream| stream.kind == StreamKind::Audio)
    {
        Ok(TrackKind::Audio)
    } else {
        Err(engine::EditError::Unsupported(
            "media has no video or audio stream",
        ))
    }
}
fn asset_state_text(locale: Locale, state: AssetState) -> String {
    match state {
        AssetState::Probing => catalog::text(locale, Text::AssetProbing).to_owned(),
        AssetState::Ready => catalog::text(locale, Text::AssetReady).to_owned(),
        AssetState::Unsupported(reason) => {
            let reason = match reason {
                UnsupportedReason::DecoderUnavailable => {
                    catalog::text(locale, Text::UnsupportedDecoder).to_owned()
                }
                UnsupportedReason::OpenFailed => {
                    catalog::text(locale, Text::UnsupportedOpenFailed).to_owned()
                }
            };
            catalog::format(locale, Text::AssetUnsupported, &[("reason", &reason)])
        }
    }
}
fn asset_state_color(state: AssetState, dark: bool) -> Color32 {
    match state {
        AssetState::Probing => theme::secondary(dark),
        AssetState::Ready => theme::success(dark),
        AssetState::Unsupported(_) => theme::warning(dark),
    }
}
fn asset_row(
    asset: &SourceAsset,
    locale: Locale,
    size: Option<u64>,
    state: AssetState,
) -> AssetRow {
    let mut info = Vec::new();
    let video = asset
        .probe()
        .streams
        .iter()
        .find_map(|stream| stream.video.as_ref());
    let audio = asset
        .probe()
        .streams
        .iter()
        .find_map(|stream| stream.audio.as_ref());
    let media_kind = MediaKind::of(asset);
    let (kind, icon) = match media_kind {
        MediaKind::Image => (Text::Image, Icon::File),
        MediaKind::Video => (Text::Video, Icon::FilmStrip),
        MediaKind::Audio => (Text::Audio, Icon::MusicNotes),
        MediaKind::Other => (Text::Media, Icon::File),
    };
    let type_label = catalog::text(locale, kind).to_owned();
    let mut brief = vec![type_label.clone()];
    info.push(type_label);
    info.push(asset.probe().duration.map_or_else(
        || catalog::text(locale, Text::DurationUnknown).to_owned(),
        display_time,
    ));
    if let Some(size) = size {
        let size = format_media_size(locale, size);
        brief.push(size.clone());
        info.push(size);
    }
    if let Some(video) = video {
        let width = catalog::number(locale, u64::from(video.width));
        let height = catalog::number(locale, u64::from(video.height));
        info.push(catalog::format(
            locale,
            Text::AssetDimensions,
            &[("width", &width), ("height", &height)],
        ));
    }
    if let Some(rate) = asset
        .probe()
        .streams
        .iter()
        .find_map(|stream| stream.frame_rate)
    {
        let value = if rate.den() == 1 {
            catalog::signed(locale, rate.num())
        } else {
            catalog::decimal(locale, rate.num() as f64 / rate.den() as f64, 3)
        };
        info.push(catalog::format(locale, Text::AssetFps, &[("rate", &value)]));
    }
    if let Some(audio) = audio {
        let rate = catalog::number(locale, u64::from(audio.sample_rate));
        let channels = catalog::number(locale, u64::from(audio.channels));
        info.push(catalog::format(
            locale,
            Text::AssetAudio,
            &[("rate", &rate), ("channels", &channels)],
        ));
    }
    AssetRow {
        id: asset.id(),
        name: file_name(asset.path()),
        info: info.join(" · "),
        brief: brief.join(" · "),
        icon,
        state,
        duration: asset.probe().duration,
    }
}
fn format_media_size(locale: Locale, bytes: u64) -> String {
    let (value, unit) = if bytes >= 1 << 30 {
        (
            catalog::decimal(locale, bytes as f64 / (1u64 << 30) as f64, 1),
            "GB",
        )
    } else if bytes >= 1 << 20 {
        (
            catalog::decimal(locale, bytes as f64 / (1u64 << 20) as f64, 1),
            "MB",
        )
    } else if bytes >= 1 << 10 {
        (
            catalog::decimal(locale, bytes as f64 / (1u64 << 10) as f64, 1),
            "KB",
        )
    } else {
        (catalog::number(locale, bytes), "B")
    };
    catalog::format(locale, Text::MediaSize, &[("size", &value), ("unit", unit)])
}
fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}
fn parse_color(value: &str) -> Result<project::Color> {
    let value = value.trim().trim_start_matches('#');
    if !matches!(value.len(), 6 | 8) || !value.is_ascii() {
        return Err(Error::Invalid("RGBA colour"));
    }
    let mut rgba = [255u8; 4];
    for (i, byte) in rgba.iter_mut().enumerate().take(value.len() / 2) {
        *byte = u8::from_str_radix(
            value
                .get(i * 2..i * 2 + 2)
                .ok_or(Error::Invalid("RGBA colour"))?,
            16,
        )
        .map_err(|_| Error::Invalid("RGBA colour"))?;
    }
    Ok(project::Color(rgba))
}
fn parse_percent(value: &str) -> Result<f32> {
    let value: f32 = value
        .trim()
        .parse()
        .map_err(|_| Error::Invalid("percentage"))?;
    if !value.is_finite() || !(0.0..=100.0).contains(&value) {
        return Err(Error::Invalid("percentage"));
    }
    Ok(value)
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
fn consume(ctx: &egui::Context, modifiers: egui::Modifiers, key: egui::Key) -> bool {
    ctx.input_mut(|input| input.consume_key(modifiers, key))
}
fn track_height(kind: TrackKind) -> f32 {
    match kind {
        TrackKind::Video => 65.0,
        TrackKind::Audio => 50.0,
        TrackKind::Text
        | TrackKind::Sticker
        | TrackKind::Effect
        | TrackKind::Filter
        | TrackKind::Adjustment => 25.0,
    }
}
fn track_icon(kind: TrackKind) -> Icon {
    match kind {
        TrackKind::Video => Icon::FilmStrip,
        TrackKind::Audio => Icon::MusicNotes,
        TrackKind::Text => Icon::TextT,
        TrackKind::Sticker => Icon::Sticker,
        TrackKind::Effect => Icon::Sparkle,
        TrackKind::Filter => Icon::Funnel,
        TrackKind::Adjustment => Icon::SlidersHorizontal,
    }
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
fn item_label(content: &ItemParams, locale: Locale, dark: bool) -> (String, Option<[u8; 4]>) {
    match content {
        ItemParams::Video(_) => (catalog::text(locale, Text::Video).to_owned(), None),
        ItemParams::Audio(_) => (catalog::text(locale, Text::Audio).to_owned(), None),
        ItemParams::Text(params) => {
            let value = params.content.as_str();
            if value.is_empty() {
                (catalog::text(locale, Text::TextItem).to_owned(), None)
            } else {
                (display_label(value, LABEL_CHARS), None)
            }
        }
        ItemParams::Caption(_) => (catalog::text(locale, Text::Caption).to_owned(), None),
        ItemParams::Overlay(params) => {
            let [r, g, b, _] = params.color.0;
            let color = if dark { [r, g, b, 90] } else { [r, g, b, 60] };
            (catalog::text(locale, Text::Overlay).to_owned(), Some(color))
        }
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
fn item_palette(
    kind: TrackKind,
    dark: bool,
    selected: bool,
    overlay: Option<[u8; 4]>,
) -> (Color32, Color32, Color32) {
    let text = theme::text(dark);
    let (fill, border) = match kind {
        TrackKind::Video => (theme::video_fill(dark), theme::video_border(dark)),
        TrackKind::Audio => (theme::field(dark), theme::border(dark)),
        _ => (theme::hover(dark), theme::border(dark)),
    };
    let fill = overlay.map_or(fill, |[r, g, b, a]| {
        Color32::from_rgba_unmultiplied(r, g, b, a)
    });
    if selected {
        (fill, theme::accent(dark), text)
    } else {
        (fill, border, text)
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct RulerConfig {
    label_seconds: f64,
    tick_seconds: f64,
    label_multiple: i64,
}
const LABEL_FRAME_INTERVALS: [f64; 5] = [2.0, 3.0, 5.0, 10.0, 15.0];
const TICK_FRAME_INTERVALS: [f64; 6] = [1.0, 2.0, 3.0, 5.0, 10.0, 15.0];
const SECOND_INTERVALS: [f64; 14] = [
    1.0, 2.0, 3.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0,
];
/// Dynamic fit minimum: content occupies 25% of the visible width at minimum.
fn fit_zoom(viewport_width: f32, duration_seconds: f64) -> f32 {
    let width = f64::from(viewport_width.max(1.0));
    let seconds = if duration_seconds > 0.0 {
        duration_seconds
    } else {
        1.0
    };
    let fit = width * f64::from(1.0 - PADDING_MAX_RATIO) / seconds;
    if !fit.is_finite() {
        return MIN_ZOOM_FLOOR;
    }
    (fit as f32).clamp(MIN_ZOOM_FLOOR, MAX_ZOOM)
}
fn zoom_percent(zoom: f32, min: f32) -> f32 {
    let span = MAX_ZOOM - min;
    if span <= 0.0 {
        1.0
    } else {
        ((zoom - min) / span).clamp(0.0, 1.0)
    }
}
fn slider_to_zoom(position: f32, min: f32) -> f32 {
    let min = min.clamp(MIN_ZOOM_FLOOR, MAX_ZOOM);
    if MAX_ZOOM <= min {
        return min;
    }
    min * (MAX_ZOOM / min).powf(position.clamp(0.0, 1.0))
}
fn zoom_to_slider(zoom: f32, min: f32) -> f32 {
    let min = min.clamp(MIN_ZOOM_FLOOR, MAX_ZOOM);
    if MAX_ZOOM <= min {
        return 0.0;
    }
    (zoom.clamp(min, MAX_ZOOM) / min).ln() / (MAX_ZOOM / min).ln()
}
fn padding_ratio(zoom: f32, min: f32) -> f32 {
    let transition = (zoom_percent(zoom, min) / PADDING_MIN_AT_ZOOM_PERCENT).min(1.0);
    PADDING_MAX_RATIO - (PADDING_MAX_RATIO - PADDING_MIN_RATIO) * transition
}
fn padding_px(viewport_width: f32, zoom: f32, min: f32) -> f32 {
    viewport_width.max(0.0) * padding_ratio(zoom, min)
}
fn max_scroll_seconds(duration_seconds: f64, viewport_width: f32, zoom: f32, min: f32) -> f32 {
    let view = (viewport_width / zoom.max(MIN_ZOOM_FLOOR)).max(0.001);
    let padding = padding_px(viewport_width, zoom, min);
    (duration_seconds as f32 + padding / zoom - view).max(0.0)
}
/// Scroll while keeping the playhead's viewport offset; restore the pre-anchor
/// scroll when the zoom slider falls back below the threshold.
fn anchored_scroll(
    old_zoom: f32,
    new_zoom: f32,
    min: f32,
    scroll: f32,
    playhead: f32,
    pre_anchor: Option<f32>,
) -> (f32, Option<f32>) {
    let old_percent = zoom_to_slider(old_zoom, min);
    let new_percent = zoom_to_slider(new_zoom, min);
    let mut pre_anchor = pre_anchor;
    if old_percent < ZOOM_ANCHOR_THRESHOLD && new_percent >= ZOOM_ANCHOR_THRESHOLD {
        pre_anchor = Some(scroll * old_zoom);
    }
    let next = if new_percent >= ZOOM_ANCHOR_THRESHOLD {
        let offset = (playhead - scroll) * old_zoom;
        playhead - offset / new_zoom
    } else if old_percent >= ZOOM_ANCHOR_THRESHOLD && new_percent < ZOOM_ANCHOR_THRESHOLD {
        pre_anchor
            .take()
            .map_or(scroll * old_zoom / new_zoom, |pixels| pixels / new_zoom)
    } else {
        scroll * old_zoom / new_zoom
    };
    (next.max(0.0), pre_anchor)
}
/// Pointer-distance-proportional edge auto-scroll in logical pixels per frame.
fn edge_auto_scroll(
    pointer: Option<egui::Pos2>,
    body: Rect,
    scroll_seconds: f32,
    max_scroll_seconds: f32,
) -> f32 {
    let Some(pointer) = pointer else {
        return 0.0;
    };
    if !body.contains(pointer) || body.width() <= 0.0 {
        return 0.0;
    }
    let relative = pointer.x - body.left();
    if relative < EDGE_AUTO_SCROLL_ZONE && scroll_seconds > 0.0 {
        let intensity = (1.0 - relative / EDGE_AUTO_SCROLL_ZONE).clamp(0.0, 1.0);
        -EDGE_AUTO_SCROLL_MAX_STEP * intensity
    } else if relative > body.width() - EDGE_AUTO_SCROLL_ZONE && scroll_seconds < max_scroll_seconds
    {
        let distance = (body.width() - relative).max(0.0);
        let intensity = (1.0 - distance / EDGE_AUTO_SCROLL_ZONE).clamp(0.0, 1.0);
        EDGE_AUTO_SCROLL_MAX_STEP * intensity
    } else {
        0.0
    }
}
fn ruler_config(zoom: f32, rate: FrameRate) -> RulerConfig {
    let fps = rate.as_f64();
    let pixels_per_second = f64::from(zoom.max(MIN_ZOOM_FLOOR));
    let pixels_per_frame = pixels_per_second / fps;
    let label_seconds = optimal_ruler_interval(
        pixels_per_frame,
        pixels_per_second,
        fps,
        f64::from(RULER_LABEL_SPACING),
        &LABEL_FRAME_INTERVALS,
    );
    let raw_tick = optimal_ruler_interval(
        pixels_per_frame,
        pixels_per_second,
        fps,
        f64::from(RULER_TICK_SPACING),
        &TICK_FRAME_INTERVALS,
    );
    let tick = tick_dividing_label(label_seconds, raw_tick, pixels_per_second, fps);
    let multiple = (label_seconds / tick).round().max(1.0) as i64;
    RulerConfig {
        label_seconds,
        tick_seconds: label_seconds / multiple as f64,
        label_multiple: multiple,
    }
}
fn optimal_ruler_interval(
    pixels_per_frame: f64,
    pixels_per_second: f64,
    fps: f64,
    minimum_spacing: f64,
    frame_intervals: &[f64],
) -> f64 {
    for interval in frame_intervals {
        if pixels_per_frame * interval >= minimum_spacing {
            return interval / fps;
        }
    }
    for seconds in SECOND_INTERVALS {
        if pixels_per_second * seconds >= minimum_spacing {
            return seconds;
        }
    }
    let mut seconds = *SECOND_INTERVALS.last().unwrap_or(&3600.0);
    while pixels_per_second * seconds < minimum_spacing {
        seconds *= 2.0;
    }
    seconds
}
fn tick_dividing_label(label: f64, raw_tick: f64, pixels_per_second: f64, fps: f64) -> f64 {
    if divides(label, raw_tick) {
        return raw_tick;
    }
    for interval in TICK_FRAME_INTERVALS {
        let candidate = interval / fps;
        if divides(label, candidate)
            && candidate * pixels_per_second >= f64::from(RULER_TICK_SPACING)
        {
            return candidate;
        }
    }
    for candidate in SECOND_INTERVALS {
        if divides(label, candidate)
            && candidate * pixels_per_second >= f64::from(RULER_TICK_SPACING)
        {
            return candidate;
        }
    }
    label
}
fn divides(label: f64, candidate: f64) -> bool {
    if candidate <= 0.0 {
        return false;
    }
    let ratio = label / candidate;
    (ratio - ratio.round()).abs() < 1e-6
}
fn snapped_seconds(seconds: f64, rate: FrameRate) -> f64 {
    Tick::from_seconds_f64(seconds, Rounding::Nearest)
        .ok()
        .and_then(|tick| rate.snap_nearest(tick).ok())
        .map_or(seconds, |tick| tick.seconds())
}
fn drop_frame(rate: FrameRate) -> bool {
    matches!(rate, FrameRate::FPS_29_97 | FrameRate::FPS_59_94)
}
fn ruler_label(seconds: f64, rate: FrameRate) -> String {
    let tick = Tick::from_seconds_f64(seconds, Rounding::Nearest)
        .ok()
        .and_then(|tick| rate.snap_nearest(tick).ok());
    match tick {
        Some(tick) => time::format_timecode(tick, rate, drop_frame(rate))
            .unwrap_or_else(|_| display_time(tick)),
        None => display_time(Tick::ZERO),
    }
}
fn display_time(at: Tick) -> String {
    let s = at.seconds().max(0.0);
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        (s / 3600.0) as u64,
        (s / 60.0) as u64 % 60,
        s as u64 % 60,
        ((s.fract() * 1000.0).floor()) as u64
    )
}
/// Compact grid badge: total minutes and seconds, matching the reference.
fn display_short_time(at: Tick) -> String {
    let seconds = at.seconds().max(0.0) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(left: f32, right: f32) -> bool {
        (left - right).abs() <= 0.01_f32.max(right.abs() * 1e-5)
    }

    #[test]
    fn ruler_ticks_keep_labels_separated_at_every_zoom_and_rate() {
        for rate in [
            FrameRate::FPS_23_976,
            FrameRate::FPS_29_97,
            FrameRate::FPS_30,
            FrameRate::FPS_60,
            FrameRate::FPS_59_94,
            FrameRate::new(1, 10).unwrap(),
            FrameRate::new(TICKS_PER_SECOND, 1).unwrap(),
        ] {
            for factor in [0.1_f32, 1.0, 10.0, 100.0] {
                let zoom = DEFAULT_ZOOM * factor;
                let config = ruler_config(zoom, rate);
                assert!(config.tick_seconds > 0.0);
                assert!(config.label_multiple >= 1);
                assert!(config.label_seconds * rate.as_f64() >= 1.0);
                assert!(
                    (config.label_seconds - config.tick_seconds * config.label_multiple as f64)
                        .abs()
                        < 1e-6,
                    "labels must land on ticks"
                );
                let label_px = config.label_seconds as f32 * zoom;
                let tick_px = config.tick_seconds as f32 * zoom;
                assert!(label_px >= RULER_LABEL_SPACING - 0.05);
                assert!(tick_px >= RULER_TICK_SPACING - 0.05);
                for offset in [0.0, 30_000_000.0] {
                    let first = (offset / config.label_seconds).floor() * config.label_seconds;
                    let gap = (snapped_seconds(first + config.label_seconds, rate)
                        - snapped_seconds(first, rate))
                        * f64::from(zoom);
                    assert!(
                        gap >= 70.0,
                        "visible timecode labels must remain separated at long durations: {gap}"
                    );
                }
            }
        }
        let df = ruler_config(DEFAULT_ZOOM, FrameRate::FPS_29_97);
        assert!(df.label_multiple >= 1);
        assert!(ruler_label(1.0, FrameRate::FPS_29_97).contains(';'));
        assert!(ruler_label(1.0, FrameRate::FPS_30).contains(':'));
        assert!(ruler_label(60.0, FrameRate::FPS_59_94).contains(';'));
        assert!(!ruler_label(60.0, FrameRate::FPS_23_976).contains(';'));
    }

    #[test]
    fn fit_zoom_uses_a_quarter_of_narrow_and_wide_viewports() {
        for width in [320.0_f32, 720.0, 1600.0] {
            let duration = 42.0;
            let min = fit_zoom(width, duration);
            assert!(close((duration * f64::from(min)) as f32, width * 0.25));
        }
        assert_eq!(fit_zoom(16_000.0, 1.0), MAX_ZOOM);
        assert_eq!(fit_zoom(1600.0, 0.001), MAX_ZOOM);
        assert_eq!(fit_zoom(1600.0, 0.0), DEFAULT_ZOOM * 10.0);
        let duration = Tick(i64::MAX).seconds();
        assert!(fit_zoom(320.0, duration).is_finite());
        assert!(max_scroll_seconds(duration, 320.0, MAX_ZOOM, MIN_ZOOM_FLOOR).is_finite());
        // A long timeline can fit below 0.1x; a short one above it.
        assert!(fit_zoom(320.0, 3600.0) < DEFAULT_ZOOM * 0.1);
        assert!(fit_zoom(1600.0, 4.0) > DEFAULT_ZOOM * 0.1);
    }

    #[test]
    fn zoom_padding_moves_from_three_quarters_to_fifteen_percent() {
        let min = fit_zoom(1000.0, 10.0);
        assert!(close(padding_ratio(min, min), PADDING_MAX_RATIO));
        let at_transition = min + (MAX_ZOOM - min) * PADDING_MIN_AT_ZOOM_PERCENT;
        assert!(close(padding_ratio(at_transition, min), PADDING_MIN_RATIO));
        assert!(close(padding_ratio(MAX_ZOOM, min), PADDING_MIN_RATIO));
        assert!(close(padding_px(1000.0, min, min), 750.0));
    }

    #[test]
    fn slider_is_exponential_and_round_trips_and_zoom_steps_multiply() {
        let min = fit_zoom(1000.0, 10.0);
        assert!(close(slider_to_zoom(0.0, min), min));
        assert!(close(slider_to_zoom(1.0, min), MAX_ZOOM));
        assert!(close(slider_to_zoom(2.0, min), MAX_ZOOM));
        assert!(close(slider_to_zoom(-1.0, min), min));
        let midpoint = slider_to_zoom(0.5, min);
        assert!(close(midpoint, (min * MAX_ZOOM).sqrt()));
        for position in [0.0_f32, 0.1, 0.35, 0.8, 1.0] {
            let zoom = slider_to_zoom(position, min);
            assert!(close(zoom_to_slider(zoom, min), position));
        }
        let start = DEFAULT_ZOOM * 3.0;
        let stepped = start * ZOOM_STEP;
        assert!(close(stepped / ZOOM_STEP, start));
        assert_eq!((MAX_ZOOM * ZOOM_STEP).min(MAX_ZOOM), MAX_ZOOM);
    }

    #[test]
    fn playhead_anchor_holds_offset_then_restores_pre_anchor_scroll() {
        let min = fit_zoom(1000.0, 40.0);
        let before = slider_to_zoom(0.1, min);
        let after = slider_to_zoom(0.35, min);
        let playhead = 20.0;
        let scroll = 2.0;
        let (held, saved) = anchored_scroll(before, after, min, scroll, playhead, None);
        let expected = playhead - (playhead - scroll) * before / after;
        assert!(close(held, expected));
        assert_eq!(saved, Some(scroll * before));
        let (restored, saved) = anchored_scroll(after, before, min, held, playhead, saved);
        assert!(close(restored, scroll));
        assert_eq!(saved, None);
        // Below the threshold the same pixel offset is kept.
        let (scaled, _) = anchored_scroll(min, before, min, 3.0, playhead, None);
        assert!(close(scaled, 3.0 * min / before));
        for target in [MAX_ZOOM, before, min] {
            let (scroll, _) = anchored_scroll(after, target, min, held, 0.0, saved);
            assert!(scroll.is_finite() && scroll >= 0.0);
        }
    }

    #[test]
    fn scroll_clamps_to_content_and_padding_bounds() {
        let min = fit_zoom(800.0, 12.0);
        assert!(close(max_scroll_seconds(12.0, 800.0, min, min), 0.0));
        let max = max_scroll_seconds(12.0, 800.0, DEFAULT_ZOOM, min);
        assert!(max > 0.0);
        assert!(close(1000.0_f32.clamp(0.0, max), max));
        assert!(close((-5.0_f32).clamp(0.0, max), 0.0));
        assert!(close(max_scroll_seconds(0.0, 800.0, min, min), 0.0));
    }

    #[test]
    fn edge_auto_scroll_is_proportional_capped_and_stops_at_bounds() {
        let body = Rect::from_min_size(egui::pos2(10.0, 10.0), Vec2::new(400.0, 200.0));
        let at = |x: f32| Some(egui::pos2(x, 100.0));
        assert!(close(
            edge_auto_scroll(at(10.0), body, 100.0, 1000.0),
            -EDGE_AUTO_SCROLL_MAX_STEP
        ));
        assert!(close(edge_auto_scroll(at(60.0), body, 100.0, 1000.0), -7.5));
        assert!(close(
            edge_auto_scroll(at(410.0), body, 100.0, 1000.0),
            15.0
        ));
        assert!(close(edge_auto_scroll(at(360.0), body, 100.0, 1000.0), 7.5));
        assert_eq!(edge_auto_scroll(at(9.0), body, 100.0, 1000.0), 0.0);
        assert_eq!(edge_auto_scroll(at(411.0), body, 100.0, 1000.0), 0.0);
        assert_eq!(edge_auto_scroll(at(10.0), body, 0.0, 1000.0), 0.0);
        assert_eq!(edge_auto_scroll(at(410.0), body, 1000.0, 1000.0), 0.0);
        assert_eq!(edge_auto_scroll(None, body, 100.0, 1000.0), 0.0);
    }
}
