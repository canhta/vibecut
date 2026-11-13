//! Applies the timeline workflows.
mod apply;
mod fades;
mod interaction;
mod item_drag;
mod media_drag;
mod viewport;
mod volume;

pub(in crate::editor) use apply::{
    apply, command_on_selection, discard_gesture, discard_timeline_preview, release_graph,
};
pub(super) use viewport::{
    anchored_scroll, edge_auto_scroll, fit_zoom, max_scroll_seconds, slider_to_zoom, zoom_to_slider,
};
