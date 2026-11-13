//! Downloadable sticker collections: pinned emoji and icon files fetched only on user action,
//! verified, recorded with their licence and searched by name and keyword.
mod collections;
mod downloads;
mod entries;
mod sanitize;
mod search;
mod store;

pub use collections::{Category, Collection, Entry, curated_collections};
pub use sanitize::{MAX_SVG_BYTES, check_svg};
pub use search::{FETCHED_SIZE, Item, Page, Unavailable};
pub use store::{CollectionState, StickerStore, is_offline};
