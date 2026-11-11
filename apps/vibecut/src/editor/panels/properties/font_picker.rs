//! Presents the engine font catalog for the text Font field: source tabs,
//! search, favourites, explicit downloads and typed preview states.
mod backend;
mod state;
mod view;

pub use backend::{Backend, EngineBackend, FontChoice};
pub(in crate::editor) use state::PendingImport;
pub use state::{Category, Download, Inspection, State, Status};
pub(in crate::editor) use view::{Action, show};
