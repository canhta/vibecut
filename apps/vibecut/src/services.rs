//! Shared services below the windows: transport, preview, crop, AI access, AI task progress and job progress.
pub mod ai_access;
pub mod ai_task;
pub mod color_picker;
pub mod crop;
pub mod music;
pub mod playback;
pub mod preview;
pub mod progress;
pub(crate) mod remote_models;
pub mod text_clipboard;
