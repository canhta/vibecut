//! Shared services below the windows: transport, preview, crop, AI access, AI task progress, job progress, the editor session, asset intake, clipboard reads and the GPU adapters.
pub mod ai_access;
pub mod ai_task;
pub mod assets;
pub mod clipboard;
pub mod color_picker;
pub mod crop;
pub mod gpu;
pub mod lifecycle;
pub mod playback;
pub mod preview;
pub mod progress;
pub(crate) mod remote_models;
