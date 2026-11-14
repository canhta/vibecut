//! Stores the shared editor presentation context.

use super::panels::player::{PreviewGuide, TextCaret};
use super::{
    Arc, AssetId, BTreeMap, Clock, Duration, Icon, ItemId, KeybindingIssue, MediaSort, MediaView,
    PathBuf, Playback, Preferences, SelectionScope, SelectionScopes, Text, Tick, WidgetState,
    actions, egui, theme,
};

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
    /// The action's label in the editor's current locale.
    pub label: String,
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
    /// Items whose keyframe lanes are open, in stable id order.
    pub expanded_lanes: Vec<ItemId>,
    pub zoom: f32,
    /// Persisted/logical horizontal offset in seconds at the left edge.
    pub scroll: f32,
    /// Dynamic fit minimum for the current viewport and content.
    pub min_zoom: f32,
    pub snapping: bool,
    /// Active snap target tick while a preview gesture holds one; `None` hides
    /// the indicator and proves snapping chose no candidate.
    pub snap_indicator: Option<Tick>,
    /// Guides of the preview transform gesture in progress, canvas pixels.
    pub preview_guides: Vec<PreviewGuide>,
    /// Bookmark whose marker editor is open.
    pub bookmark_popover: Option<project::BookmarkId>,
    /// Command id of the candidate staged by the group preview contract.
    pub preview: Option<&'static str>,
    /// Latest move/trim refusal shown in the timeline.
    pub timeline_refusal: Option<String>,
    /// Landing hint for a move drag (`track:N`, `insert_above:N`, `insert_below:N`).
    pub drop_zone: Option<String>,
    pub ripple: bool,
    pub preview_zoom: f32,
    pub preview_fit: bool,
    pub preview_offset: egui::Vec2,
    pub preview_fullscreen: bool,
    /// The preview axis toggle state.
    pub preview_axis: bool,
    /// The pointer time the timeline guide holds, if any.
    pub hover_time: Option<Tick>,
    /// The tick the shared preview last requested; equals `hover_time` while
    /// the axis scrubs and is `None` when the player follows the playhead.
    pub preview_at: Option<Tick>,
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
    pub import_error: Option<Arc<dyn engine::Failure + 'static>>,
    pub import_toast: Option<String>,
    pub media_selection: Vec<AssetId>,
    pub assets: Vec<AssetInspection>,
    pub effect_previews: BTreeMap<String, super::assets::effects::EffectPreviewStatus>,
    pub media_view: MediaView,
    pub media_sort: MediaSort,
    pub media_order: Vec<AssetId>,
    pub media_highlight: Option<AssetId>,
    /// A clip whose Replace media request is waiting for the window's native picker.
    pub replace_request: Option<ItemId>,
    pub media_delete_pending: Vec<AssetId>,
    pub sounds: Option<super::assets::sounds::SoundsView>,
    pub music: Option<super::assets::sounds::music::MusicView>,
    pub voiceover: super::dialogs::voiceover::VoiceoverView,
    pub export: Option<crate::export::ExportInspection>,
    pub export_options: crate::export::ExportOptions,
    /// Working crop while its dialog is open; the project keeps its own copy.
    pub crop: Option<super::panels::player::crop::CropInspection>,
    pub models: Option<crate::model_manager::ModelInspection>,
    pub subtitles: Option<super::assets::captions::Inspection>,
    /// Unknown actions, invalid chords and conflicts from the persisted map.
    pub keybinding_issues: Vec<KeybindingIssue>,
    pub color_preview: Option<project::Color>,
    /// Clip whose chroma-key eyedropper is armed for the next player click.
    pub key_color_pick: Option<ItemId>,
    /// Clip and mode of the armed matte refinement brush.
    pub matte_brush: Option<(ItemId, project::MatteStrokeMode)>,
    /// Present while the shortcuts dialog is open.
    pub shortcuts: Option<ShortcutsInspection>,
    /// Font picker state, tab, favourites and download.
    pub font_picker: super::properties::font_picker::Inspection,
    /// Present while an inline text session is open.
    pub text_caret: Option<TextCaret>,
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
pub(super) struct AssetRow {
    pub(super) id: AssetId,
    pub(super) name: String,
    pub(super) info: String,
    pub(super) icon: Icon,
    pub(super) state: super::assets::AssetState,
    pub(super) duration: Option<Tick>,
}
/// App state only. The immutable engine project is read through Session, never copied
/// into a parallel frontend project model. Every edit dispatches its registry ID.
pub struct Editor {
    pub session: crate::services::lifecycle::EditorSession,
    pub playback: Playback,
    pub preferences: Preferences,
    /// Workspace-owned dialogs take keyboard priority over the editor's sheets.
    pub(crate) overlay_open: bool,
    pub selection_scopes: SelectionScopes,
    pub(super) properties: super::properties::State,
    pub(super) frame: super::frame::State,
    pub(super) player: super::panels::player::State,
    pub(super) timeline: super::timeline::State,
    pub(super) error: Option<String>,
    /// Journal revision last seen by the header and the local `HH:MM:SS` it was first seen.
    pub(super) autosaved: Option<(u64, String)>,
    pub(crate) persistence_error: Option<std::sync::Arc<engine::recovery::Error>>,
    pub(super) configured: bool,
    pub(crate) lifecycle_action: Option<crate::services::lifecycle::EditorAction>,
    /// Time base for transient UI timers (toast and media reveal).
    pub(super) clock: Arc<dyn Clock>,
    pub(super) toast: Option<Toast>,
    pub(super) assets: super::assets::State,
    pub(super) dialogs: super::dialogs::State,
    /// A copy-snapshot action waits for the next frame's egui context.
    pub(super) copy_snapshot_requested: bool,
    /// Cross-area requests the frame drains after the area that raised them.
    pub(super) requests: Vec<super::Request>,
    /// Runner for `export.clip` downloads, created by the first download.
    pub(super) clip_downloads: Option<engine::jobs::JobRunner>,
    /// The download job the runner is waiting on.
    pub(super) clip_job: Option<engine::jobs::JobId>,
}
