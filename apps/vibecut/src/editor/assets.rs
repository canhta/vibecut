//! Owns the assets presentation boundary.
mod view;
mod workflow;
pub(super) use view::show;

use super::{
    Align2, Arc, AssetId, AssetRow, AssetState, AssetsTab, BTreeMap, BTreeSet, BUTTON_ICON,
    Color32, Command, CreatedId, Duration, Editor, Error, FontId, Icon, Id, ImportEvent,
    ImportFailure, ImportProbe, ImportQueue, ItemParams, Locale, MEDIA_CARD_GAP, MEDIA_CARD_HEIGHT,
    MEDIA_CARD_WIDTH, MEDIA_REVEAL, MEDIA_ROW_HEIGHT, MediaKind, MediaMarquee, MediaReveal,
    MediaSortKey, MediaSortOrder, MediaView, NewItem, PathBuf, Rect, Result, SelectionScopes,
    SourceAsset, SourceRange, StreamKind, TICKS_PER_SECOND, Text, ThumbnailOutcome,
    ThumbnailSource, Tick, Toast, TrackKind, UnsupportedReason, Vec2, catalog, display_short_time,
    display_time, effect_preview_params, egui, error_detail, icons, media_panel, theme,
};

use workflow::{asset_kind, asset_row, asset_state_color, asset_state_text, file_name};
