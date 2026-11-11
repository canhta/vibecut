//! Supplies native command fixtures through a shared test interface.
mod project;
pub(crate) use project::{
    add_track, exec, info, new_project, overlay_project, overlay_project_color,
};
mod media;
pub(super) use media::{decoded_mono, decoded_rgb, generate_audio, video_oetf};
