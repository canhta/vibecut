//! Owns the assets presentation boundary.
pub mod captions;
pub mod effects;
pub mod import;
pub mod media_panel;
pub mod music;
mod projection;
pub mod sounds;
mod state;
mod tabs;
mod transition;
mod view;
mod workflow;
pub use import::{
    ImportEvent, ImportFailure, ImportProbe, ImportQueue, MAX_IMPORT_FILES, ThumbnailOutcome,
    ThumbnailSource,
};
pub use media_panel::{MediaKind, MediaSort, MediaSortKey, MediaSortOrder, MediaView};
pub(in crate::editor) use projection::prepare;
pub use state::AssetState;
pub(in crate::editor) use state::State;
pub use tabs::AssetsTab;
pub(in crate::editor) use view::{delete_dialog, drop_overlay, import_overlay, toast};

/// The Audio tab's two pages. Sound effects keep the existing provider; Music
/// adds the open-source search section.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum AudioSection {
    Music,
    #[default]
    Sounds,
}

use super::super::{
    Align2, Arc, AssetId, AssetRow, BTreeMap, BTreeSet, Color32, Command, ContentHash, CreatedId,
    Editor, Error, FontId, Icon, Id, ItemParams, Locale, NewItem, PathBuf, Rect, Result,
    SelectionScopes, SourceAsset, SourceRange, StreamKind, TICKS_PER_SECOND, Text, Tick, TrackKind,
    UnsupportedReason, Vec2, egui, error_detail, icons, theme,
};
use state::MediaMarquee;
use workflow::{asset_kind, asset_row, asset_state_color, file_name};

/// Read-only workflow projection owned by this area's state.
#[derive(Default)]
struct Model {
    preferences: crate::preferences::Preferences,
    rows: Vec<Row>,
    thumbnails: BTreeMap<AssetId, egui::TextureHandle>,
    effects: Vec<EffectRow>,
    transition_refusal: Option<Text>,
    transition_drag_enabled: bool,
    graphic_insertion_ready: bool,
    graphic_previews: [Option<egui::TextureHandle>; 4],
    importing: bool,
    selection: BTreeSet<AssetId>,
    reveal: Option<AssetId>,
    marquee: Option<MediaMarquee>,
    failed: bool,
    sounds: Option<sounds::SoundsView>,
    music: Option<music::MusicView>,
    /// Full project-derived music attribution records, independent of Saved.
    music_credits: Vec<String>,
    /// Set when the project's credit text exceeds its bounded aggregate.
    music_credits_failed: bool,
}

impl Model {
    /// Tooltip text from the action's first effective persisted chord.
    fn shortcut(&self, action: &str) -> Option<String> {
        crate::editor::context::shortcut_text(&self.preferences.keybindings, action)
    }
}

pub(in crate::editor) fn show(
    state: &mut State,
    cx: &crate::editor::Context<'_>,
    ui: &mut egui::Ui,
) -> Vec<Action> {
    let mut model = std::mem::take(&mut state.model);
    model.preferences.locale = cx.locale;
    model.preferences.dark = cx.dark;
    let actions = view::show(state, &model, ui);
    state.model = model;
    actions
}
struct Row {
    display: AssetRow,
    kind: MediaKind,
}
struct EffectRow {
    kind: &'static str,
    name: &'static str,
    shelf: project::EffectShelf,
    status: Option<effects::EffectPreviewStatus>,
    texture: Option<egui::TextureHandle>,
}
pub(in crate::editor) struct Recorded {
    pub(in crate::editor) id: String,
    pub(in crate::editor) response: egui::Response,
    pub(in crate::editor) glyph: Option<Icon>,
    pub(in crate::editor) tooltip: Option<String>,
}
/// Intentions routed by the frame after painting has released its read borrow.
pub(in crate::editor) enum Action {
    SelectTab(AssetsTab),
    Command(String),
    Select(AssetId, egui::Modifiers),
    Delete(Vec<AssetId>),
    Record(Box<Recorded>),
    Marquee {
        content: Rect,
        background: Box<egui::Response>,
        rects: Vec<(AssetId, Rect)>,
    },
    Captions(Rect),
    EffectDrag(&'static str),
    AddEffect(&'static str),
    RetryEffect(&'static str),
    TransitionDrag(&'static str),
    AddTransition(&'static str),
    SoundsMode(sounds::SoundsMode),
    SoundsSearch {
        more: bool,
    },
    SoundsCommercial(bool),
    SoundsPreview(u64),
    SoundsToggleSaved(u64),
    SoundsAdd(u64),
    SoundsCancelCache,
    SoundsRetry,
    MusicMode(music::MusicMode),
    MusicChip(&'static str),
    MusicSearch {
        more: bool,
    },
    MusicPreview(String),
    MusicToggleSaved(String),
    MusicAdd(String),
    MusicCancelCache,
    MusicRetry,
    MusicRetryAdd,
    MusicCredits,
    MusicKeyApply,
    MusicKeyRemove,
    /// Apply or delete one app-owned adjustment preset.
    ApplyAdjustment(usize),
    DeleteAdjustment(usize),
}
