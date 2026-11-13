//! The byte source behind fetched stickers (`noto:`, `lucide:`, `phosphor:`): the composition lends
//! one, render parses what it returns once per id and rasterises at the destination size.

use super::sticker::{FLAGS_PROVIDER, StickerError};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

/// Most parsed trees kept; the curated collections hold fewer files than this.
const MAX_TREES: usize = 1024;

/// Why a lent source has no bytes for an id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StickerFault {
    /// No collection of the source owns this id.
    Unknown,
    /// The id belongs to a collection whose file is not in the download cache.
    NotDownloaded,
    /// The cached file failed its size, hash or safety check.
    Corrupt,
}

/// Verified SVG bytes of fetched stickers, lent by the process composition. An implementation
/// returns bytes only after its own integrity and safety checks.
pub trait StickerSource: Send + Sync {
    /// The SVG bytes of a `<collection>:<value>` id.
    ///
    /// # Errors
    ///
    /// A [`StickerFault`] naming why the id has no usable bytes.
    fn svg(&self, sticker: &str) -> Result<Vec<u8>, StickerFault>;
}

fn source() -> &'static RwLock<Option<Arc<dyn StickerSource>>> {
    static SOURCE: std::sync::OnceLock<RwLock<Option<Arc<dyn StickerSource>>>> =
        std::sync::OnceLock::new();
    SOURCE.get_or_init(RwLock::default)
}

fn trees() -> &'static Mutex<BTreeMap<String, Arc<resvg::usvg::Tree>>> {
    static TREES: std::sync::OnceLock<Mutex<BTreeMap<String, Arc<resvg::usvg::Tree>>>> =
        std::sync::OnceLock::new();
    TREES.get_or_init(Mutex::default)
}

/// Lends the byte source of fetched stickers to every renderer in the process, replacing any
/// earlier one and dropping the trees parsed from it.
pub fn lend_sticker_source(lent: Arc<dyn StickerSource>) {
    *source().write().unwrap_or_else(PoisonError::into_inner) = Some(lent);
    trees()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
}

/// The parsed tree of a fetched sticker. Each id parses once per lend; failures are not cached,
/// so a download that finishes later takes effect on the next frame.
pub(super) fn fetched_tree(sticker: &str) -> Result<Arc<resvg::usvg::Tree>, StickerError> {
    let fail = |kind: fn(String) -> StickerError| kind(sticker.to_owned());
    let missing = || fail(|sticker| StickerError::Missing { sticker });
    let provider = sticker.split_once(':').map(|(provider, _)| provider);
    if provider.is_none_or(|provider| provider == FLAGS_PROVIDER) {
        return Err(missing());
    }
    if let Some(tree) = trees()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(sticker)
    {
        return Ok(Arc::clone(tree));
    }
    let lent = source()
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .ok_or_else(missing)?;
    let bytes = lent.svg(sticker).map_err(|fault| match fault {
        StickerFault::Unknown => missing(),
        StickerFault::NotDownloaded => fail(|sticker| StickerError::NotDownloaded { sticker }),
        StickerFault::Corrupt => fail(|sticker| StickerError::Invalid { sticker }),
    })?;
    let tree = Arc::new(
        resvg::usvg::Tree::from_data(&bytes, &resvg::usvg::Options::default())
            .map_err(|_| fail(|sticker| StickerError::Invalid { sticker }))?,
    );
    let mut trees = trees().lock().unwrap_or_else(PoisonError::into_inner);
    if trees.len() >= MAX_TREES {
        trees.clear();
    }
    trees.insert(sticker.to_owned(), Arc::clone(&tree));
    Ok(tree)
}

/// Refuses a fetched sticker whose file is not in the cache or fails verification. An id no
/// source owns keeps the placeholder contract of [`StickerError::Missing`] and passes.
///
/// # Errors
///
/// [`StickerError::NotDownloaded`] or [`StickerError::Invalid`].
pub(crate) fn admit(sticker: &str) -> Result<(), StickerError> {
    match fetched_tree(sticker) {
        Ok(_) | Err(StickerError::Missing { .. }) => Ok(()),
        Err(error) => Err(error),
    }
}
