//! Stores the assets panel's media and import state.
use super::{
    BTreeMap, BTreeSet, ContentHash, ImportFailure, ImportQueue, Model, PathBuf, Text, egui,
};
use engine::control::StickerCategory;
use media::UnsupportedReason;
use std::time::Duration;

/// Highlight lifetime for `media.reveal`; presentation state only.
pub(in crate::editor) const MEDIA_REVEAL: Duration = Duration::from_millis(1500);

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

#[derive(Clone, Copy, Debug)]
pub(in crate::editor) struct MediaReveal {
    pub(in crate::editor) asset: super::AssetId,
    pub(in crate::editor) until: Duration,
}

#[derive(Clone, Debug)]
pub(in crate::editor) struct MediaMarquee {
    pub(in crate::editor) origin: egui::Pos2,
    pub(in crate::editor) before: BTreeSet<super::AssetId>,
    pub(in crate::editor) additive: bool,
    pub(in crate::editor) moved: bool,
}

pub(in crate::editor) struct State {
    pub(super) model: Model,
    pub(super) searches: BTreeMap<super::AssetsTab, String>,
    pub(super) filter: Filter,
    pub(super) effect_shelf: Option<project::EffectShelf>,
    /// Effects tab: the tile preview owner and the drag in flight.
    pub(in crate::editor) effect_previews: Option<super::effects::EffectPreviews>,
    pub(in crate::editor) effect_drag: Option<String>,
    pub(super) transition_family: Option<&'static str>,
    pub(super) transition_previews: BTreeMap<&'static str, super::transition::Preview>,
    pub(in crate::editor) transition_drag: Option<(&'static str, std::sync::Arc<project::Project>)>,
    /// Sounds tab: the presentation owner and the add target directory.
    pub(in crate::editor) sounds: Option<super::sounds::Sounds>,
    pub(in crate::editor) sound_media_dir: Option<PathBuf>,
    pub(super) sounds_mode: super::sounds::SoundsMode,
    pub(super) sounds_commercial: bool,
    pub(in crate::editor) audio_section: super::AudioSection,
    pub(super) music_mode: super::music::MusicMode,
    pub(super) music_chip: &'static str,
    pub(super) music_search: String,
    /// Masked key draft; never inspected, logged or persisted.
    pub(super) music_key: String,
    pub(super) music_settings: bool,
    pub(super) music_credits_open: bool,
    /// Media tab: the asset selection and the rows' backing caches.
    pub(in crate::editor) media_selection: BTreeSet<super::AssetId>,
    pub(in crate::editor) media_anchor: Option<super::AssetId>,
    pub(in crate::editor) media_sizes: BTreeMap<super::AssetId, (ContentHash, Option<u64>)>,
    pub(in crate::editor) media_marquee: Option<MediaMarquee>,
    pub(in crate::editor) media_reveal: Option<MediaReveal>,
    pub(in crate::editor) media_delete_confirm: Option<Vec<super::AssetId>>,
    pub(in crate::editor) asset_states: BTreeMap<super::AssetId, AssetState>,
    pub(in crate::editor) thumbnails:
        BTreeMap<super::AssetId, engine::thumbnails::RetainedThumbnail>,
    pub(in crate::editor) thumbnail_textures:
        std::cell::RefCell<BTreeMap<super::AssetId, egui::TextureHandle>>,
    /// Import tab: the running batch and its last terminal report.
    pub(in crate::editor) imports: Option<ImportQueue>,
    pub(in crate::editor) import_total: usize,
    pub(in crate::editor) import_completed: usize,
    pub(in crate::editor) import_imported: usize,
    pub(in crate::editor) import_cancelled: bool,
    pub(in crate::editor) import_failures: Vec<ImportFailure>,
    pub(in crate::editor) import_notice: Option<String>,
    pub(in crate::editor) import_error: Option<std::sync::Arc<dyn engine::Failure + 'static>>,
    pub(in crate::editor) pending_imports: BTreeMap<super::AssetId, PathBuf>,
    pub(in crate::editor) restoring_asset: Option<super::AssetId>,
    pub(in crate::editor) thumbnail_root: Option<PathBuf>,
    /// Captions tab: the panel, built on first use.
    pub(in crate::editor) captions: Option<super::captions::Subtitles>,
    /// Stickers tab: the category tab, whether See all widened the results, and the thumbnails.
    pub(super) sticker_category: StickerCategory,
    pub(super) sticker_expanded: bool,
    pub(in crate::editor) sticker_thumbnails: super::stickers::Thumbnails,
}

impl Default for State {
    fn default() -> Self {
        Self {
            model: Model::default(),
            searches: BTreeMap::new(),
            filter: Filter::default(),
            effect_shelf: None,
            effect_previews: None,
            effect_drag: None,
            transition_family: None,
            transition_previews: BTreeMap::new(),
            transition_drag: None,
            sounds: None,
            sound_media_dir: None,
            sounds_mode: super::sounds::SoundsMode::Discover,
            sounds_commercial: true,
            audio_section: super::AudioSection::Sounds,
            music_mode: super::music::MusicMode::Browse,
            music_chip: libraries::music::CHIPS[1],
            music_search: String::new(),
            music_key: String::new(),
            music_settings: false,
            music_credits_open: false,
            media_selection: BTreeSet::new(),
            media_anchor: None,
            media_sizes: BTreeMap::new(),
            media_marquee: None,
            media_reveal: None,
            media_delete_confirm: None,
            asset_states: BTreeMap::new(),
            thumbnails: BTreeMap::new(),
            thumbnail_textures: std::cell::RefCell::default(),
            imports: None,
            import_total: 0,
            import_completed: 0,
            import_imported: 0,
            import_cancelled: false,
            import_failures: Vec::new(),
            import_notice: None,
            import_error: None,
            pending_imports: BTreeMap::new(),
            restoring_asset: None,
            thumbnail_root: None,
            captions: None,
            sticker_category: StickerCategory::default(),
            sticker_expanded: false,
            sticker_thumbnails: Default::default(),
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Filter {
    #[default]
    All,
    Video,
    Audio,
    Image,
}
impl Filter {
    pub(super) const ALL: [Self; 4] = [Self::All, Self::Video, Self::Audio, Self::Image];
    pub(super) fn label(self) -> Text {
        match self {
            Self::All => Text::AssetsAll,
            Self::Video => Text::Video,
            Self::Audio => Text::Audio,
            Self::Image => Text::Image,
        }
    }
    pub(super) fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Image => "image",
        }
    }
}
