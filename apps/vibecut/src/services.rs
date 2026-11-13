//! Shared services below the windows: transport, preview, crop, AI access, AI task progress, job progress, the editor session, asset intake and the GPU adapters.
pub mod ai_access;
pub mod ai_task;
pub mod assets;
pub mod color_picker;
pub mod crop;
pub mod gpu;
pub mod lifecycle;
pub mod model_cache;
pub mod playback;
pub mod preview;
pub mod progress;
pub(crate) mod remote_models;
pub mod text_clipboard;
