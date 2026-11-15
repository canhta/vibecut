//! Owns the assets presentation boundary.
pub mod captions;
mod catalog;
pub mod effects;
pub mod import;
pub mod media_panel;
mod projection;
pub mod sounds;
mod state;
mod stickers;
mod tabs;
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
pub(in crate::editor) use view::{
    delete_dialog, drop_overlay, import_overlay, rename_dialog, toast,
};

/// The Audio tab's two pages. Sound effects keep the existing provider; Music
/// adds the open-source search section.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum AudioSection {
    Music,
    #[default]
    Sounds,
}

/// The Media tab's two pages, chosen in its sub-navigation: the asset browser or the
/// project's scene list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum MediaSection {
    #[default]
    Media,
    Timeline,
    /// The Yours group's pages, in the reference's order.
    Favorites,
    MyPresets,
    BrandAssets,
}

/// One folder tile of the Media page.
#[derive(Clone, Debug, PartialEq, Eq)]
struct FolderRow {
    id: project::MediaFolderId,
    name: String,
    count: usize,
}

/// One scene row on the Media sub-navigation's Timeline page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::editor) struct SceneRow {
    pub(in crate::editor) id: project::SceneId,
    pub(in crate::editor) name: String,
    pub(in crate::editor) main: bool,
    pub(in crate::editor) active: bool,
}

/// The Captions tab's four pages, chosen in its sub-navigation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum CaptionsSection {
    #[default]
    Auto,
    Add,
    Translate,
    Chapters,
}

use super::super::{
    Align2, Arc, AssetId, AssetRow, BTreeMap, BTreeSet, Color32, Command, ContentHash, CreatedId,
    Error, FontId, Icon, Id, ItemParams, Locale, NewItem, PathBuf, Rect, Result, SourceAsset,
    SourceRange, StreamKind, TICKS_PER_SECOND, Text, Tick, TrackKind, UnsupportedReason, Vec2,
    egui, error_detail, icons, theme,
};
use state::MediaMarquee;
use workflow::{asset_kind, asset_row, asset_state_color, file_name};

/// Read-only workflow projection owned by this area's state.
#[derive(Default)]
struct Model {
    preferences: crate::preferences::Preferences,
    rows: Vec<Row>,
    /// Media folders in project order; empty off the Media tab.
    folders: Vec<FolderRow>,
    thumbnails: BTreeMap<AssetId, egui::TextureHandle>,
    effects: Vec<EffectRow>,
    filters: Vec<FilterRow>,
    transition_refusal: Option<Text>,
    transition_drag_enabled: bool,
    insertion_ready: bool,
    /// Stickers tab rows; present only while that tab is shown.
    stickers: Option<stickers::StickerRows>,
    /// Media Timeline page rows, in project order; empty on the other pages.
    scenes: Vec<SceneRow>,
    /// Text tab: the selected text items a style tile applies to; empty on the other tabs.
    text_targets: Vec<project::ItemId>,
    importing: bool,
    selection: BTreeSet<AssetId>,
    clip_export: Option<media_panel::clips::ClipStatus>,
    reveal: Option<AssetId>,
    marquee: Option<MediaMarquee>,
    failed: bool,
    sounds: Option<sounds::SoundsView>,
    music: Option<sounds::music::MusicView>,
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
    /// The folder holding the asset, when it sits in one.
    folder: Option<project::MediaFolderId>,
}
struct EffectRow {
    kind: &'static str,
    name: &'static str,
    shelf: project::EffectShelf,
    status: Option<effects::EffectPreviewStatus>,
    texture: Option<egui::TextureHandle>,
}
/// One Filters tile: a look from the catalogue and its rendered preview.
struct FilterRow {
    id: &'static str,
    name: &'static str,
    shelf: project::FilterShelf,
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
    /// Open the Rename dialog for one asset.
    RenameMedia(AssetId),
    MediaRenameCancel,
    MediaRenameConfirm,
    /// Group these assets into a new folder.
    NewFolder(Vec<AssetId>),
    ExportClips(Vec<AssetId>),
    ClipExportCancel,
    Record(Box<Recorded>),
    Marquee {
        content: Rect,
        background: Box<egui::Response>,
        rects: Vec<(AssetId, Rect)>,
    },
    Captions(Rect),
    EffectDrag(&'static str),
    /// A press on asset cards turned into a drag of these assets.
    MediaDrag(Vec<AssetId>),
    AddEffect(&'static str),
    RetryEffect(&'static str),
    /// Insert one filter clip at the playhead.
    AddFilter(&'static str),
    TransitionDrag(&'static str),
    AddTransition(&'static str),
    SoundsMode(sounds::SoundsMode),
    SoundsSearch {
        more: bool,
    },
    SoundsCommercial(bool),
    SoundsPreview(u64),
    /// Preview a favourite that is no longer listed, from the URL its star stored.
    SoundsPlay(libraries::sounds::SoundResult),
    SoundsToggleSaved(u64),
    SoundsAdd(u64),
    SoundsCancelCache,
    SoundsRetry,
    MusicMode(sounds::music::MusicMode),
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
    /// Ask the lending owner to open a store whose open failed.
    LibraryRetry(sounds::LibraryStore),
    MusicCredits,
    MusicKeyApply,
    MusicKeyRemove,
    /// Insert an adjustment layer (an effect item) at the playhead.
    InsertAdjustment,
    /// Apply or delete one app-owned adjustment preset.
    ApplyAdjustment(usize),
    DeleteAdjustment(usize),
    /// Switch the open scene from the Media Timeline page.
    SwitchScene(project::SceneId),
    /// Apply one built-in look to the selected text items (`text.apply_preset`).
    ApplyTextLook(project::TextLook),
    /// Insert one built-in template at the playhead (`text.insert_template`).
    InsertTextTemplate(project::TextTemplate),
    /// Insert one bundled flag at the playhead and record it as recent.
    InsertSticker(String),
    /// Insert one shape through its `graphic.insert.*` command and record it as recent.
    InsertShape(String),
    ClearStickerRecents,
    /// Star or unstar one catalogue tile by its id within the family.
    ToggleFavorite {
        family: &'static str,
        id: String,
        favorite: bool,
    },
    /// Star or unstar one sound; a star stores the metadata its Favorites row shows.
    ToggleSoundFavorite {
        sound: crate::preferences::SoundFavorite,
        favorite: bool,
    },
    /// Download one Emoji or Icons collection on the user's explicit action; its card shows the
    /// licence credit before the button.
    DownloadSticker(String),
    CancelStickerDownload,
    /// Read the active brand kit again (the Brand assets page's refresh icon).
    RefreshBrand,
    /// Copy one brand kit file into the project and insert it at the playhead.
    InsertBrandMedia(u64),
    /// A press on a brand kit row started a drag; the timeline resolves where it lands.
    BrandDrag(u64),
}
