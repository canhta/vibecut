//! Stores the shared editor presentation context.

use super::{
    Arc, AssetId, BTreeMap, BTreeSet, BookmarkId, Clock, ContentHash, Duration, EyedropperState,
    Icon, ImportFailure, ImportQueue, ItemId, KeybindingIssue, Keymap, MediaSort, MediaView,
    PathBuf, Playback, Preferences, Rect, SelectionScope, SelectionScopes, SourceRange, Text, Tick,
    TrackId, UnsupportedReason, Vec2, actions, egui, preview, theme,
};

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
/// The shortcuts dialog's four tabs. The registry's eight categories are
/// grouped beneath them so every category stays reachable in one place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutTab {
    Timeline,
    Player,
    Basic,
    Other,
}
impl ShortcutTab {
    pub const ALL: &'static [Self] = &[Self::Timeline, Self::Player, Self::Basic, Self::Other];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Timeline => "timeline",
            Self::Player => "player",
            Self::Basic => "basic",
            Self::Other => "other",
        }
    }

    pub const fn label(self) -> Text {
        match self {
            Self::Timeline => Text::ShortcutsTabTimeline,
            Self::Player => Text::ShortcutsTabPlayer,
            Self::Basic => Text::ShortcutsTabBasic,
            Self::Other => Text::ShortcutsTabOther,
        }
    }

    pub const fn categories(self) -> &'static [actions::ActionCategory] {
        match self {
            Self::Timeline => &[
                actions::ActionCategory::Timeline,
                actions::ActionCategory::Selection,
            ],
            Self::Player => &[
                actions::ActionCategory::Playback,
                actions::ActionCategory::Navigation,
            ],
            Self::Basic => &[
                actions::ActionCategory::File,
                actions::ActionCategory::Edit,
                actions::ActionCategory::History,
            ],
            Self::Other => &[actions::ActionCategory::View],
        }
    }

    pub fn for_category(category: actions::ActionCategory) -> Self {
        Self::ALL
            .iter()
            .copied()
            .find(|tab| tab.categories().contains(&category))
            .unwrap_or(Self::Other)
    }
}
// Shared measured typography and control geometry; provenance lives in the token owner.
pub(super) const SHORTCUT_TITLE_SIZE: f32 = theme::type_size::BODY;
pub(super) const SHORTCUT_TEXT_SIZE: f32 = theme::type_size::BODY;
pub(super) const SHORTCUT_ROW_HEIGHT: f32 = theme::size::SHORTCUT_ROW;
pub(super) const SHORTCUT_CONTROL_HEIGHT: f32 = theme::size::FIELD;

/// One shortcuts-dialog row. Keys are already platform glyphs, and an action
/// with no effective chord stays listed as [`Self::unbound`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutRow {
    pub action: String,
    pub category: &'static str,
    pub tab: ShortcutTab,
    pub keys: Vec<String>,
    pub customized: bool,
    pub unbound: bool,
}

/// Transient shortcuts-dialog state; bindings themselves live in preferences.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutsInspection {
    pub tab: ShortcutTab,
    pub recording: Option<String>,
    pub error: Option<String>,
    pub rows: Vec<ShortcutRow>,
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
    /// Active snap target tick while a preview gesture holds one; `None` hides
    /// the indicator and proves snapping chose no candidate.
    pub snap_indicator: Option<Tick>,
    pub ripple: bool,
    pub preview_zoom: f32,
    pub preview_fit: bool,
    pub preview_offset: egui::Vec2,
    pub preview_fullscreen: bool,
    pub timecode: String,
    pub timecode_editing: bool,
    pub snapshot_dialog_open: bool,
    pub snapshot_saved: Option<PathBuf>,
    pub snapshot_error: Option<engine::control::ErrorCode>,
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
    /// Unknown actions, invalid chords and conflicts from the persisted map.
    pub keybinding_issues: Vec<KeybindingIssue>,
    pub color_preview: Option<project::Color>,
    pub eyedropper: Option<EyedropperState>,
    /// Present while the shortcuts dialog is open.
    pub shortcuts: Option<ShortcutsInspection>,
}
#[derive(Clone, Debug)]
pub(super) struct PreferenceCommand {
    pub(super) key: String,
    pub(super) before: serde_json::Value,
    pub(super) after: serde_json::Value,
}
impl PreferenceCommand {
    pub(in crate::editor) fn new(
        key: &str,
        before: serde_json::Value,
        after: serde_json::Value,
    ) -> Self {
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
pub(super) struct PreferenceHistory {
    pub(super) undo: Vec<PreferenceCommand>,
    pub(super) redo: Vec<PreferenceCommand>,
}
impl PreferenceHistory {
    pub(in crate::editor) fn record(&mut self, command: PreferenceCommand) {
        self.undo.push(command);
        self.redo.clear();
    }
    pub(in crate::editor) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub(in crate::editor) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub(in crate::editor) fn undo(&mut self, preferences: &mut Preferences) -> bool {
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
    pub(in crate::editor) fn redo(&mut self, preferences: &mut Preferences) -> bool {
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
pub(super) struct Toast {
    pub(super) text: String,
    pub(super) detail: Option<String>,
    pub(super) until: Duration,
}
#[derive(Clone, Debug)]
pub(super) struct TimecodeEdit {
    pub(super) text: String,
    pub(super) focus: bool,
}
#[derive(Clone, Debug)]
pub(super) struct AssetRow {
    pub(super) id: AssetId,
    pub(super) name: String,
    pub(super) info: String,
    pub(super) brief: String,
    pub(super) icon: Icon,
    pub(super) state: AssetState,
    pub(super) duration: Option<Tick>,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct MediaReveal {
    pub(super) asset: AssetId,
    pub(super) until: Duration,
}
#[derive(Clone, Debug)]
pub(super) struct MediaMarquee {
    pub(super) origin: egui::Pos2,
    pub(super) before: BTreeSet<AssetId>,
    pub(super) additive: bool,
    pub(super) moved: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DragMode {
    Move,
    TrimStart,
    TrimEnd,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct DragOrigin {
    pub(super) item: ItemId,
    pub(super) track: TrackId,
    pub(super) start: Tick,
    pub(super) end: Tick,
}
#[derive(Clone, Debug)]
pub(super) struct DragState {
    pub(super) item: ItemId,
    pub(super) mode: DragMode,
    pub(super) press_x: f32,
    pub(super) origins: Vec<DragOrigin>,
    pub(super) source_range: SourceRange,
    pub(super) speed: time::Speed,
    pub(super) moved: bool,
}
#[derive(Clone, Debug)]
pub(super) struct EffectReorder {
    pub(super) item: ItemId,
    pub(super) effect: String,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct BookmarkDrag {
    pub(super) bookmark: BookmarkId,
    pub(super) press_x: f32,
    pub(super) start: Tick,
    pub(super) moved: bool,
}
/// One scrub field's unit contract. Pixels per step are recorded in the app
/// README; the inventory leaves the exact sensitivity unverified.
#[derive(Clone, Copy, Debug)]
pub(super) struct ScrubSpec {
    pub(super) field: &'static str,
    pub(super) label: Text,
    pub(super) min: f64,
    pub(super) max: f64,
    pub(super) step: f64,
    pub(super) pixels_per_step: f32,
    pub(super) decimals: usize,
    pub(super) suffix: &'static str,
}
impl ScrubSpec {
    pub(in crate::editor) fn quantize(self, value: f64) -> f64 {
        if !value.is_finite() {
            return self.min;
        }
        let steps = ((value.clamp(self.min, self.max) - self.min) / self.step).round();
        (self.min + steps * self.step).clamp(self.min, self.max)
    }
    pub(in crate::editor) fn format(self, value: f64) -> String {
        format!("{:.*}", self.decimals, self.quantize(value))
    }
}
pub(super) const OPACITY_SCRUB: ScrubSpec = ScrubSpec {
    field: "opacity",
    label: Text::Opacity,
    min: 0.0,
    max: 1.0,
    step: 0.01,
    pixels_per_step: 4.0,
    decimals: 2,
    suffix: "",
};
pub(super) const TEXT_SIZE_SCRUB: ScrubSpec = ScrubSpec {
    field: "text_size",
    label: Text::TextSize,
    min: 1.0,
    max: 4096.0,
    step: 1.0,
    pixels_per_step: 2.0,
    decimals: 0,
    suffix: "",
};
pub(super) const SPEED_SCRUB: ScrubSpec = ScrubSpec {
    field: "speed",
    label: Text::Speed,
    min: 0.01,
    max: 5.0,
    step: 0.01,
    pixels_per_step: 2.0,
    decimals: 2,
    suffix: "x",
};
pub(super) const AUDIO_GAIN_SCRUB: ScrubSpec = ScrubSpec {
    field: "audio_gain",
    label: Text::AudioGain,
    min: -60.0,
    max: 20.0,
    step: 0.01,
    pixels_per_step: 4.0,
    decimals: 2,
    suffix: "dB",
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ScrubTarget {
    Item(&'static str),
    Effect { effect: String, key: String },
}
impl ScrubTarget {
    pub(in crate::editor) fn draft_key(&self) -> String {
        match self {
            Self::Item(field) => (*field).to_owned(),
            Self::Effect { effect, key } => format!("effects.{effect}.param.{key}"),
        }
    }
    pub(in crate::editor) fn widget_id(&self) -> String {
        match self {
            Self::Item(field) => format!("inspector.{field}.scrub"),
            Self::Effect { effect, key } => format!("effects.{effect}.param.{key}.scrub"),
        }
    }
}
#[derive(Clone, Debug)]
pub(super) struct ScrubDrag {
    pub(super) item: ItemId,
    pub(super) target: ScrubTarget,
    pub(super) start: f64,
    pub(super) last: f64,
}
/// App state only. The immutable engine project is read through Session, never copied
/// into a parallel frontend project model. Every edit dispatches its registry ID.
pub struct Editor {
    pub session: crate::lifecycle::EditorSession,
    pub playback: Playback,
    pub preferences: Preferences,
    pub(super) preference_history: PreferenceHistory,
    pub(super) keymap: Keymap,
    pub(super) keymap_source: actions::KeybindingPreferences,
    pub(super) keybinding_issues: Vec<KeybindingIssue>,
    /// Voiceover recording owner (p1-parity/86) sets this; global keys wait.
    pub(super) recording_active: bool,
    /// Workspace-owned dialogs take keyboard priority over the editor's sheets.
    pub(crate) overlay_open: bool,
    pub selection: BTreeSet<ItemId>,
    pub selection_scopes: SelectionScopes,
    pub(super) marquee: Option<(egui::Pos2, BTreeSet<ItemId>, bool)>,
    pub(super) properties: super::properties::State,
    pub(super) effect_previews: Option<crate::effect_previews::EffectPreviews>,
    pub(super) effect_textures: BTreeMap<String, egui::TextureHandle>,
    pub(super) effect_drag: Option<String>,
    pub(super) effect_drop_targets: Vec<(ItemId, Rect)>,
    pub ripple: bool,
    pub(super) zoom: f32,
    pub(super) scroll: f32,
    pub(super) vertical_scroll: f32,
    /// Content viewport width used for the dynamic fit minimum.
    pub(super) timeline_viewport: f32,
    /// Pixel scroll captured when zoom crossed the playhead-anchor threshold.
    pub(super) pre_anchor_scroll: Option<f32>,
    pub(super) last_view: project::ViewState,
    pub(super) widgets: BTreeMap<String, WidgetState>,
    pub panels: BTreeMap<&'static str, Rect>,
    pub(super) frame: super::frame::State,
    pub(super) preview_view: preview::View,
    pub(super) preview_viewport: Vec2,
    pub(super) preview_fullscreen: bool,
    pub(super) fullscreen_commanded: bool,
    pub(super) timecode_edit: Option<TimecodeEdit>,
    pub(super) timecode_error: Option<Text>,
    pub snapshot: preview::SnapshotDialog,
    pub(super) drag: Option<DragState>,
    pub(super) bookmark_drag: Option<BookmarkDrag>,
    pub(super) snap_indicator: Option<Tick>,
    pub(super) texture: Option<egui::TextureHandle>,
    pub(super) texture_key: Option<(u64, Tick)>,
    pub(super) texture_owner: Option<std::sync::Arc<crate::playback::Frame>>,
    pub(super) error: Option<String>,
    pub(crate) persistence_error: Option<std::sync::Arc<engine::EditError>>,
    pub(super) configured: bool,
    pub(super) track_rename: Option<(TrackId, String)>,
    pub(super) track_rename_focus: bool,
    pub(super) track_drag: Option<TrackId>,
    pub(crate) lifecycle_action: Option<crate::lifecycle::EditorAction>,
    pub(super) imports: Option<ImportQueue>,
    pub(super) import_total: usize,
    pub(super) import_completed: usize,
    pub(super) import_imported: usize,
    pub(super) import_cancelled: bool,
    pub(super) import_failures: Vec<ImportFailure>,
    pub(super) import_notice: Option<String>,
    pub(super) import_error: Option<Arc<engine::EditError>>,
    /// Time base for transient UI timers (toast and media reveal).
    pub(super) clock: Arc<dyn Clock>,
    pub(super) toast: Option<Toast>,
    pub(super) media_selection: BTreeSet<AssetId>,
    pub(super) media_anchor: Option<AssetId>,
    pub(super) media_sizes: BTreeMap<AssetId, (ContentHash, Option<u64>)>,
    pub(super) media_marquee: Option<MediaMarquee>,
    pub(super) media_reveal: Option<MediaReveal>,
    pub(super) media_delete_confirm: Option<Vec<AssetId>>,
    pub(super) scenes_open: bool,
    pub(super) scene_select: bool,
    pub(super) scene_selected: BTreeSet<project::SceneId>,
    pub(super) scene_confirm: bool,
    pub(super) scene_rename: Option<(project::SceneId, String)>,
    pub(super) scene_rename_focus: bool,
    pub export: Option<crate::export::ExportDialog>,
    pub(super) shortcuts_open: bool,
    pub(super) shortcut_tab: ShortcutTab,
    pub(super) shortcut_recording: Option<&'static str>,
    pub(super) shortcut_error: Option<String>,
    pub(super) models: Option<crate::model_manager::ModelManager>,
    pub(super) models_visible: bool,
    pub(super) asset_states: BTreeMap<AssetId, AssetState>,
    pub(super) thumbnails: BTreeMap<AssetId, engine::thumbnails::RetainedThumbnail>,
    pub(super) pending_imports: BTreeMap<AssetId, PathBuf>,
    pub(super) restoring_asset: Option<AssetId>,
    pub(super) thumbnail_textures: BTreeMap<AssetId, egui::TextureHandle>,
    pub(super) thumbnail_root: Option<PathBuf>,
    pub subtitles: Option<crate::subtitles::Subtitles>,
}
