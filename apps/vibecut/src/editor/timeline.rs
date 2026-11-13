//! Owns the timeline panel: its state, the whole-panel `show` and the actions it requests.
//!
//! `show(state, cx, ui)` paints the panel (toolbar, ruler, headers, lanes,
//! scrollbars) from the shared read-only [`Context`] and returns the
//! requests the frame routes. The panel never touches the session, the
//! selection or the transport; the frame applies each [`Action`].
mod action;
mod audio;
mod graph;
mod layout;
mod proxies;
mod snapping;
mod state;
mod view;
mod visuals;
mod workflow;

pub(super) use action::{Action, Gesture};
pub(super) use state::{DropZone, State, Tool};
pub(super) use view::{TOOLBAR_ACTIONS, add_track_command, show, show_scenes};
pub(super) use visuals::Media;

use super::{
    Align2, AssetId, BTreeSet, BookmarkId, Color32, Command, Context, FontId, FrameRate, Icon, Id,
    ItemId, ItemParams, KeyframeSelection, Locale, Rect, Result, Rounding, SourceRange, Text, Tick,
    TrackId, TrackKind, Vec2, display_label, egui, icons, theme,
};

use layout::{BookmarkView, ItemView, TimelineLayout, TrackRow};
use snapping::frame_snap;
use state::{BookmarkDrag, DragMode, DragOrigin, DragState};
use view::{item_label, track_height};
use workflow::{edge_auto_scroll, max_scroll_seconds, slider_to_zoom, zoom_to_slider};

const TOOLBAR_HEIGHT: f32 = theme::size::TOOLBAR;
const BUTTON_ICON: f32 = theme::icon::DENSE;
const ICON_BUTTON: f32 = theme::size::CONTROL;
const RULER_HEIGHT: f32 = theme::size::RULER;
const LABEL_WIDTH: f32 = theme::size::TRACK_LABEL;
const SCROLLBAR: f32 = theme::size::SCROLLBAR;
const TRACK_GAP: f32 = theme::space::SM;
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
const RULER_LABEL_SPACING: f32 = 160.0;
const RULER_TICK_SPACING: f32 = 18.0;
const LABEL_CHARS: usize = 120;

#[cfg(test)]
mod tests;
