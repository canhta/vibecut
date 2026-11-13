//! Downloadable sticker collections: pinned emoji and icon files fetched only on user action,
//! verified, recorded with their licence and searched by name and keyword.
mod collections;
mod downloads;
mod entries;
mod render_source;
mod sanitize;
mod search;
mod store;

pub use collections::{Category, Collection, Entry, curated_collections};
pub use render_source::native_store;
pub use sanitize::{MAX_SVG_BYTES, check_svg};
pub use search::{FETCHED_SIZE, Item, Page, Unavailable};
pub use store::{CollectionState, StickerStore, is_offline};

/// Lowercase hex SHA-256 of `bytes`, the form the pinned collection entries record.
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
