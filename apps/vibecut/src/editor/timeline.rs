//! Owns the timeline panel: its state, the whole-panel `show` and the actions it requests.
//!
//! `show(state, cx, ui)` paints the panel (toolbar, ruler, headers, lanes,
//! scrollbars) from the shared read-only [`Context`] and returns the
//! requests the frame routes. The panel never touches the session, the
//! selection or the transport; the frame applies each [`Action`].
mod action;
mod audio;
mod layout;
mod state;
mod view;
mod visuals;
mod workflow;

pub(super) use action::{Action, Gesture};
pub(super) use state::{DropZone, State};
pub(super) use view::{TOOLBAR_ACTIONS, add_track_command, show, show_scenes};
pub(super) use visuals::Media;

use super::{
    Align2, AssetId, BTreeSet, BUTTON_ICON, BookmarkId, Color32, Command, Context, DEFAULT_ZOOM,
    EDGE_AUTO_SCROLL_MAX_STEP, EDGE_AUTO_SCROLL_ZONE, FontId, FrameRate, HORIZONTAL_WHEEL_STEP,
    ICON_BUTTON, Icon, Id, ItemId, ItemParams, KeyframeSelection, LABEL_CHARS, LABEL_WIDTH, Locale,
    MAX_ZOOM, MIN_ZOOM_FLOOR, PADDING_MAX_RATIO, PADDING_MIN_AT_ZOOM_PERCENT, PADDING_MIN_RATIO,
    RULER_HEIGHT, RULER_LABEL_SPACING, RULER_TICK_SPACING, Rect, Result, Rounding, SCROLLBAR,
    SnapCandidate, SnapSource, SourceRange, TOOLBAR_HEIGHT, TRACK_GAP, Text, Tick, TrackId,
    TrackKind, Vec2, ZOOM_ANCHOR_THRESHOLD, display_label, display_time, egui, icons, snap_point,
    theme,
};

use layout::{BookmarkView, ItemView, TimelineLayout, TrackRow};
use state::{BookmarkDrag, DragMode, DragOrigin, DragState};
use view::{item_label, track_height};
use workflow::{edge_auto_scroll, frame_snap, max_scroll_seconds, slider_to_zoom, zoom_to_slider};

#[cfg(test)]
mod tests;
