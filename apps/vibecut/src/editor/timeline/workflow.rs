//! Applies the timeline workflows.
mod fades;
mod interaction;
mod item_drag;
mod viewport;
mod volume;

pub(super) use viewport::{
    anchored_scroll, edge_auto_scroll, fit_zoom, max_scroll_seconds, slider_to_zoom, zoom_to_slider,
};
