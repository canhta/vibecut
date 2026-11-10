//! Persists managed projects across session lifetimes.
mod projects;
pub mod library {
    //! Owns managed project storage.
    pub use super::projects::{ProjectEntry, ProjectLibrary};
}
mod recovery_workflow;
pub mod recovery {
    //! Owns recoverable persistent sessions.
    pub use super::recovery_workflow::{
        Autosave, Candidate, Discovery, Journal, Schedule, discover,
    };
}
mod cover_image;
pub(crate) mod project_cover {
    //! Captures a managed project cover image.
    pub(crate) use super::cover_image::capture;
}
