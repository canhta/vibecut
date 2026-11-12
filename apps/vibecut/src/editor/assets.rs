//! Owns the assets presentation boundary.
pub mod music;
mod projection;
pub mod sounds;
mod state;
mod transition;
mod view;
mod workflow;
pub(super) use projection::prepare;
pub(super) use state::State;

/// The Audio tab's two pages. Sound effects keep the existing provider; Music
/// adds the open-source search section.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum AudioSection {
    Music,
    #[default]
    Sounds,
}

use super::{
    Align2, Arc, AssetId, AssetRow, AssetState, AssetsTab, BTreeMap, BTreeSet, Color32, Command,
    CreatedId, Duration, Editor, Error, FontId, Icon, Id, ImportEvent, ImportFailure, ImportProbe,
    ImportQueue, ItemParams, Locale, MEDIA_REVEAL, MediaKind, MediaMarquee, MediaReveal,
    MediaSortKey, MediaSortOrder, MediaView, NewItem, PathBuf, Rect, Result, SelectionScopes,
    SourceAsset, SourceRange, StreamKind, TICKS_PER_SECOND, Text, ThumbnailOutcome,
    ThumbnailSource, Tick, Toast, TrackKind, UnsupportedReason, Vec2, display_short_time,
    display_time, effect_preview_instance, egui, error_detail, icons, media_panel, theme,
};
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
    graphic_previews: [Option<egui::TextureHandle>; 2],
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

pub(super) fn show(state: &mut State, cx: &super::Context<'_>, ui: &mut egui::Ui) -> Vec<Action> {
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
    status: Option<crate::effect_previews::EffectPreviewStatus>,
    texture: Option<egui::TextureHandle>,
}
pub(super) struct Recorded {
    pub(super) id: String,
    pub(super) response: egui::Response,
    pub(super) glyph: Option<Icon>,
    pub(super) tooltip: Option<String>,
}
/// Intentions routed by the frame after painting has released its read borrow.
pub(super) enum Action {
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
}

impl Editor {
    /// Render the assets surface in isolation for offscreen design audits.
    pub fn assets_ui(&mut self, ui: &mut egui::Ui) {
        Self::fonts(ui.ctx());
        icons::install(ui.ctx());
        Self::theme(ui.ctx(), self.preferences.dark);
        crate::kit::install_editor_density(ui.ctx());
        self.frame.widgets.clear();
        self.tick();
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, self.panel_fill());
        self.frame.panels.insert("media", ui.max_rect());
        self.show_assets(ui);
        self.import_overlay(ui.ctx());
    }
}
