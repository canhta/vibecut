//! The download cache of the fetched sticker collections: what is cached, what verifies and what
//! reads back.
use super::collections::{Collection, LICENSE_FILE};
use super::sanitize::check_svg;
use engine::StickerError;
use engine::control::Result;
use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use transport::DownloadTransport;

/// Sidecar keys every cached file carries: the asset sidecar format of `open-source.md` plus the
/// collection and the commit the file was read at.
pub(super) const SIDECAR_KEYS: [&str; 10] = [
    "asset",
    "title",
    "author",
    "source",
    "license",
    "license-file",
    "added",
    "notes",
    "collection",
    "commit",
];
/// Ticket that admitted the collections, recorded in every sidecar.
pub(super) const ADMITTED_BY: &str = "p1-parity/267";

/// What the cache holds for one collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectionState {
    /// No directory: nothing was downloaded.
    NotDownloaded,
    /// Every pinned file and the licence text are present with the pinned size and a sidecar, and
    /// nothing else is in the directory.
    Ready,
    /// A directory exists but a file is missing, resized, unrecorded or foreign. The next explicit
    /// download replaces it; its bytes are never served.
    Invalid,
}

/// The sticker collection store: the curated catalog, the cache root and the transport downloads
/// use. One store serves a process.
pub struct StickerStore {
    pub(super) root: PathBuf,
    pub(super) collections: Vec<Collection>,
    pub(super) transport: Arc<dyn DownloadTransport>,
    pub(super) active: Arc<Mutex<BTreeSet<String>>>,
}

impl StickerStore {
    /// A store over `root` for the given collections. Nothing is read or created yet.
    ///
    /// # Errors
    ///
    /// [`StickerError::Catalog`] when a collection is malformed or ids repeat.
    pub fn new(
        root: PathBuf,
        collections: Vec<Collection>,
        transport: Arc<dyn DownloadTransport>,
    ) -> Result<Self> {
        let ids = collections
            .iter()
            .map(|collection| collection.id.as_str())
            .collect::<BTreeSet<_>>();
        if ids.len() != collections.len() || !collections.iter().all(Collection::validate) {
            return Err(StickerError::Catalog.into());
        }
        Ok(Self {
            root,
            collections,
            transport,
            active: Arc::new(Mutex::new(BTreeSet::new())),
        })
    }

    /// The curated collections over the per-user cache and the shared HTTPS transport.
    ///
    /// # Errors
    ///
    /// Fails when the OS cache location is unavailable.
    pub fn native() -> Result<Self> {
        Self::new(
            engine::cache::CacheRoot::discover()?.stickers(),
            super::curated_collections(),
            Arc::new(transport::UreqTransport::new()),
        )
    }

    /// The collections this store offers, in catalog order.
    pub fn collections(&self) -> &[Collection] {
        &self.collections
    }

    pub(super) fn collection(&self, id: &str) -> Option<&Collection> {
        self.collections
            .iter()
            .find(|collection| collection.id == id)
    }

    /// Whether the collection is cached. This lists the directory and compares sizes and sidecar
    /// presence; hashes are checked when a file is read, so the call is cheap enough for a panel
    /// refresh.
    ///
    /// # Errors
    ///
    /// [`StickerError::NotDownloadable`] for an unknown collection; IO failures other than a
    /// missing directory.
    pub fn state(&self, collection: &str) -> Result<CollectionState> {
        let collection = self
            .collection(collection)
            .ok_or(StickerError::NotDownloadable)?;
        let dir = self.root.join(&collection.id);
        match fs::symlink_metadata(&dir) {
            Ok(meta) if meta.is_dir() => Ok(if shape_matches(&dir, collection)? {
                CollectionState::Ready
            } else {
                CollectionState::Invalid
            }),
            Ok(_) => Ok(CollectionState::Invalid),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok(CollectionState::NotDownloaded)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// The verified SVG bytes of a `<collection>:<value>` id: size, SHA-256 and the sticker
    /// admission check all pass, and the sidecar exists.
    ///
    /// # Errors
    ///
    /// [`StickerError::NotCached`] for an unknown id or a missing file;
    /// [`StickerError::Integrity`] for changed bytes or a missing sidecar.
    pub fn read(&self, sticker_id: &str) -> Result<Vec<u8>> {
        let (provider, value) =
            super::super::parse_sticker_id(sticker_id).ok_or(StickerError::NotCached)?;
        let collection = self.collection(provider).ok_or(StickerError::NotCached)?;
        let entry = collection.entry(value).ok_or(StickerError::NotCached)?;
        let file = file_path(&self.root, collection, &entry.value);
        let bytes = match fs::read(&file) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(StickerError::NotCached.into());
            }
            Err(error) => return Err(error.into()),
        };
        if bytes.len() as u64 != entry.size
            || super::sha256_hex(&bytes) != entry.sha256
            || !has_sidecar(&file)
        {
            return Err(StickerError::Integrity.into());
        }
        check_svg(&bytes)?;
        Ok(bytes)
    }
}

/// Whether an error is the transport being unreachable, so a panel can show the offline state
/// instead of a failure.
pub fn is_offline(error: &engine::control::Error) -> bool {
    matches!(
        error,
        engine::control::Error::Runtime(engine::RuntimeError::NetworkTransient)
    )
}

pub(super) fn file_path(root: &Path, collection: &Collection, value: &str) -> PathBuf {
    root.join(&collection.id).join(format!("{value}.svg"))
}

pub(super) fn sidecar_path(file: &Path) -> PathBuf {
    let mut path = file.as_os_str().to_os_string();
    path.push(".attribution");
    PathBuf::from(path)
}

fn has_sidecar(file: &Path) -> bool {
    fs::read_to_string(sidecar_path(file)).is_ok_and(|text| {
        SIDECAR_KEYS.iter().all(|key| {
            text.lines().any(|line| {
                line.strip_prefix(&format!("{key}:"))
                    .is_some_and(|value| !value.trim().is_empty())
            })
        })
    })
}

/// The directory holds exactly the expected files, each a regular file of the pinned size with a
/// sidecar; the licence text has its own sidecar.
pub(super) fn shape_matches(dir: &Path, collection: &Collection) -> Result<bool> {
    let mut expected: std::collections::BTreeMap<String, u64> = collection
        .entries
        .iter()
        .map(|entry| (format!("{}.svg", entry.value), entry.size))
        .collect();
    expected.insert(LICENSE_FILE.to_owned(), collection.license_size);
    let mut seen = BTreeSet::new();
    for item in fs::read_dir(dir)? {
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        if let Some(base) = name.strip_suffix(".attribution") {
            if !expected.contains_key(base) {
                return Ok(false);
            }
            continue;
        }
        let Some(size) = expected.get(&name) else {
            return Ok(false);
        };
        let meta = fs::symlink_metadata(item.path())?;
        if !meta.is_file() || meta.len() != *size || !has_sidecar(&item.path()) {
            return Ok(false);
        }
        seen.insert(name);
    }
    Ok(seen.len() == expected.len())
}
