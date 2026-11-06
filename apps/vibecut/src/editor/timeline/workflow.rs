//! Applies the timeline workflows.
mod interaction;
mod item_drag;
mod snapping;
mod viewport;

pub(super) use viewport::{edge_auto_scroll, max_scroll_seconds, slider_to_zoom, zoom_to_slider};
