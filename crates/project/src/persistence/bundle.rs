//! Self-contained project bundles: a directory holding the envelope, a manifest and hashed media copies.

mod export;
mod files;
mod import;
mod manifest;

pub use export::{ExportedBundle, export};
pub use import::{ImportedBundle, import};
pub use manifest::{BUNDLE_FORMAT_ID, MAX_BUNDLE_BYTES, MAX_MEDIA_BYTES};
