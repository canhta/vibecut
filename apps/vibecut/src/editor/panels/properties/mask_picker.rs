//! Presents the clip mask row's sub-panel: the shape grid, its fields and Done.
mod state;
mod view;

pub(in crate::editor) use state::current;
pub use state::{Choice, State};
pub(in crate::editor) use view::{Action, Timing, show};
