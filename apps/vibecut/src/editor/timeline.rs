//! Owns the timeline presentation boundary.
mod view;
mod workflow;
pub(super) use view::show;

use super::{
    Align2, Arc, AssetId, BTreeSet, BUTTON_ICON, BookmarkDrag, BookmarkId, Color32, Command,
    CreatedId, DragMode, DragOrigin, DragState, EDGE_AUTO_SCROLL_MAX_STEP, EDGE_AUTO_SCROLL_ZONE,
    Editor, FontId, FrameRate, HORIZONTAL_WHEEL_STEP, ICON_BUTTON, Icon, Id, ItemId, ItemParams,
    LABEL_CHARS, LABEL_WIDTH, Locale, MAX_ZOOM, MIN_ZOOM_FLOOR, PADDING_MAX_RATIO,
    PADDING_MIN_AT_ZOOM_PERCENT, PADDING_MIN_RATIO, RULER_HEIGHT, RULER_LABEL_SPACING,
    RULER_TICK_SPACING, Rect, Result, Rounding, SCROLLBAR, SnapCandidate, SnapSource,
    TOOLBAR_HEIGHT, TOOLBAR_SPACING, TRACK_GAP, Text, Tick, TrackId, TrackKind, Vec2,
    ZOOM_ANCHOR_THRESHOLD, catalog, display_label, display_time, drop_frame, egui, icons,
    snap_point, theme,
};

mod state;
use state::{BookmarkView, ItemView, TimelineLayout, TrackRow};
use view::{item_label, track_height};
use workflow::{edge_auto_scroll, slider_to_zoom, zoom_to_slider};

pub(super) use view::show_scenes;

use workflow::max_scroll_seconds;
