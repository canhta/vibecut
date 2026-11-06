//! Applies the timeline workflows.
mod interaction;
mod item_drag;
mod snapping;
mod viewport;

pub(super) use snapping::frame_snap;
pub(super) use viewport::{
    anchored_scroll, edge_auto_scroll, fit_zoom, max_scroll_seconds, slider_to_zoom, zoom_to_slider,
};
