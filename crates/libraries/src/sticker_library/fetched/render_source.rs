//! The renderer's view of the store: verified bytes of downloaded stickers, and the process store
//! that is lent to it.
use super::store::StickerStore;
use engine::StickerError;
use engine::control::Error;
use render::{StickerFault, StickerSource};
use std::sync::{Arc, OnceLock};

impl StickerSource for StickerStore {
    fn svg(&self, sticker: &str) -> Result<Vec<u8>, StickerFault> {
        if self.resolve(sticker).is_none() {
            return Err(StickerFault::Unknown);
        }
        self.read(sticker)
            .map(with_default_colour)
            .map_err(|error| match error {
                Error::Sticker(StickerError::NotCached) => StickerFault::NotDownloaded,
                _ => StickerFault::Corrupt,
            })
    }
}

/// White, the default colour of a fetched sticker. Lucide and Phosphor draw `currentColor`, which
/// resolves to black unless the root names a colour; the root gets `color="#FFFFFF"` so the tile
/// and the inserted sticker read on the dark panel. A root that names its own colour keeps it.
/// Only the lent bytes change; the cached file keeps its verified bytes.
fn with_default_colour(bytes: Vec<u8>) -> Vec<u8> {
    let Some((at, attribute)) = default_colour_insert(&bytes) else {
        return bytes;
    };
    let (head, tail) = bytes.split_at(at);
    let mut coloured = Vec::with_capacity(bytes.len() + attribute.len());
    coloured.extend_from_slice(head);
    coloured.extend_from_slice(attribute);
    coloured.extend_from_slice(tail);
    coloured
}

/// Where the white root colour goes and its bytes, when the root names no colour and is a plain
/// `svg` element: before the root's first attribute, after its namespace declarations, or just
/// after `<svg` when it has no attribute. `None` leaves the bytes unchanged.
fn default_colour_insert(bytes: &[u8]) -> Option<(usize, &'static [u8])> {
    let text = std::str::from_utf8(bytes).ok()?;
    let document = roxmltree::Document::parse(text).ok()?;
    let root = document.root_element();
    let start = root.range().start;
    if root.attribute("color").is_some() || text.get(start..start + 4) != Some("<svg") {
        return None;
    }
    Some(match root.attributes().next() {
        Some(first) => (first.range().start, b"color=\"#FFFFFF\" "),
        None => (start + 4, b" color=\"#FFFFFF\""),
    })
}

/// The per-user store over the curated collections, opened once per process and lent to the
/// renderer on first use, so preview and export read the same verified cache the panel, the CLI
/// and MCP control search. `None` when the host has no cache location; only flags exist then.
pub fn native_store() -> Option<&'static StickerStore> {
    static STORE: OnceLock<Option<Arc<StickerStore>>> = OnceLock::new();
    STORE
        .get_or_init(|| {
            let store = Arc::new(StickerStore::native().ok()?);
            render::lend_sticker_source(store.clone());
            Some(store)
        })
        .as_deref()
}
