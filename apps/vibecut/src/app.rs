//! Composition root: the window registry, the stores it lends and the control attach.
mod chrome;
mod control;
mod geometry;
mod lending;
/// Other platforms keep the in-window header menu only.
#[cfg(target_os = "macos")]
mod native_menu;
mod picker;
mod registry;
mod runs;
mod viewports;
mod window;
mod work;

pub use control::{background, poll_workspace, sticker_lend};
pub use registry::Workspace;
pub use window::EditorWindow;
