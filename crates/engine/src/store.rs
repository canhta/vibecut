//! Persists managed projects across session lifetimes.
mod projects;
pub mod library {
    //! Managed project catalog. External-file projects continue to use Autosave directly.
    pub use super::projects::{ProjectEntry, ProjectLibrary};
}
mod recovery_workflow;
pub mod recovery {
    //! Revision-bound session journals. Only the worker serializes project snapshots.
    pub use super::recovery_workflow::{
        Autosave, Candidate, Discovery, Journal, Schedule, discover,
    };
}
mod cover_image;
pub(crate) mod project_cover {
    //! Captures a managed project cover image.
    pub(crate) use super::cover_image::{Hooks, capture};
}
