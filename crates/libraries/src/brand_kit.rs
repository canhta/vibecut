//! Keeps the user's brand kits outside projects; a project copies a kit value when it uses one.
mod archive;
mod control;
mod files;
mod record;
mod store;

pub use record::{MAX_ENTRIES, MAX_FILE_BYTES, MAX_KITS, MAX_NAME_BYTES};
pub use store::{BrandKit, KitSummary};

/// Result of every brand kit operation.
pub type Result<T> = std::result::Result<T, engine::BrandKitError>;
