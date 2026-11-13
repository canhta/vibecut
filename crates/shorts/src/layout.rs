//! Chooses the layout of each shot of a short and composes it from crop and transform params.
mod choice;
mod composition;
mod overrides;
mod pieces;
mod plan;

pub(crate) use composition::{Geometry, Slot};
pub use overrides::{Layout, LayoutEdit, LayoutOverrides};
pub use plan::ShotLayout;
pub(crate) use plan::plan;
