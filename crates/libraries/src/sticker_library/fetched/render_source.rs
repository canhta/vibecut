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
        self.read(sticker).map_err(|error| match error {
            Error::Sticker(StickerError::NotCached) => StickerFault::NotDownloaded,
            _ => StickerFault::Corrupt,
        })
    }
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
