//! Composition root: the window registry, the stores it lends and the control attach.
mod chrome;
mod control;
mod geometry;
mod lending;
mod picker;
mod registry;
mod runs;
mod viewports;
mod window;
mod work;

pub use control::{background, poll_workspace, sticker_lend};
pub use registry::Workspace;
pub use window::EditorWindow;
