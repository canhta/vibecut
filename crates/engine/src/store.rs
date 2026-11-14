//! Persists managed projects across session lifetimes.
mod lut_storage;
mod projects;
pub mod luts {
    //! Durable user-imported LUT bundles, owning copied source assets.
    pub use super::lut_storage::{Error, LutStore, Result};
}
pub mod library {
    //! Owns managed project storage.
    pub use super::projects::{
        Error, MAX_RUNS, ProjectEntry, ProjectLibrary, RETENTION_DAYS, Result, ShortsCard,
        UnreadableProject,
    };
}
mod recovery_workflow;
pub mod recovery {
    //! Owns recoverable persistent sessions.
    pub use super::recovery_workflow::{
        Autosave, Candidate, Discovery, Error, Journal, Result, Schedule, discover,
    };
}
mod cover_image;
pub mod cover {
    //! Renders a project cover frame at the size an adapter asks for.
    pub use super::cover_image::{Image, render};
}
pub(crate) mod project_cover {
    //! Captures a managed project cover image.
    pub(crate) use super::cover_image::capture;
    pub use super::cover_image::{Lookup, uncorrected};
}
