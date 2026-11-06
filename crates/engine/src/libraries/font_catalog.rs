//! Resolves font assets for project rendering.
use crate::cache::{ArtifactKind, CacheKey, KeyInputs};
use crate::jobs::ResourceClass;
use crate::jobs::{
    CancelToken, JobBytes, JobContext, JobId, JobKind, JobRunner, JobSpec, JobStage,
};
use crate::{EditError, Result};
use ai::transport::DownloadTransport;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

const BUFFER: usize = 64 * 1024;
/// Job reservation: the streaming buffer plus hashing state.
const WORK_BYTES: u64 = 256 * 1024;
/// Most faces one scan reports; further faces are ignored.
pub const MAX_SCANNED_FACES: usize = 50_000;
/// Most faces one family may register in a single load.
const MAX_FAMILY_FACES: usize = 64;
/// Admitting ticket recorded in downloaded-font attribution sidecars.
const ADMITTED_BY: &str = "p1-parity-28";
/// Pin of the curated upstream tree; every URL and hash below belongs to this commit.
const GOOGLE_FONTS_COMMIT: &str = "eb20e9cf19d97cf579fa3867d729d4222d6c59e1";

/// Where a family comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FontOrigin {
    /// Shipped inside the application with a licence record.
    Bundled,
    /// Installed in the operating system.
    System,
    /// Curated Google Fonts family, downloadable on explicit request.
    Google,
}

/// Whether a family can render now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FontState {
    Available,
    /// A catalog family whose files are not in the verified local cache.
    NotDownloaded,
}

/// One family in a listing.
#[derive(Clone, Debug, PartialEq, Serialize, schemars::JsonSchema)]
pub struct FontEntry {
    pub family: String,
    pub origin: FontOrigin,
    /// Style names such as `Regular` or `Bold Italic`, sorted by weight.
    pub styles: Vec<String>,
    pub state: FontState,
    /// SPDX licence id of bundled and Google families.
    pub license: Option<String>,
    /// Upstream source URL of bundled and Google families.
    pub source: Option<String>,
    /// Bytes `fonts.download` would fetch, for families that are not downloaded.
    pub download_bytes: Option<u64>,
}

/// Outcome of loading a family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FontLoadState {
    /// Faces are registered and layout resolves the family.
    Loaded,
    /// Known catalog family with no cached files; layout falls back to the default family.
    NotDownloaded,
    /// Cached files failed their pinned hash or size; they are not used until downloaded again.
    CacheInvalid,
    /// No bundled, system or catalog family has this name; layout falls back.
    Unknown,
}

/// Result of [`FontStore::load`].
#[derive(Clone, Debug, PartialEq, Serialize, schemars::JsonSchema)]
pub struct FontLoad {
    pub family: String,
    pub state: FontLoadState,
    pub origin: Option<FontOrigin>,
    /// The family layout will use: the request when loaded, otherwise the default family.
    pub effective_family: String,
    pub styles: Vec<String>,
}

/// Font catalog failures that are not a load state.
#[derive(Debug, thiserror::Error)]
pub enum FontError {
    #[error("the font store is not available in this session")]
    Unavailable,
    #[error("family is not a downloadable catalog font")]
    NotDownloadable,
    #[error("a download for this family is already running")]
    DownloadActive,
    #[error("downloaded bytes do not match the catalog size or SHA-256")]
    Integrity,
    #[error("font server did not answer with a complete file")]
    Response,
    #[error("font file cannot be registered")]
    Face(#[from] text::FontError),
    #[error("font catalog entry is invalid")]
    Catalog,
}

/// A face found by a scan of installed fonts.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ScannedFace {
    pub family: String,
    pub weight: u16,
    pub italic: bool,
    pub path: PathBuf,
    pub index: u32,
}

/// A pinned, downloadable font file.
#[derive(Clone, Debug)]
pub struct GoogleFile {
    /// Plain file name (no separators) used inside the cache directory.
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub weight: u16,
    pub italic: bool,
}

/// A curated downloadable family with its licence record.
#[derive(Clone, Debug)]
pub struct GoogleFamily {
    pub family: String,
    /// Lowercase directory name inside the cache.
    pub slug: String,
    pub author: String,
    /// Upstream page for the family at the pinned commit.
    pub source: String,
    /// SPDX id.
    pub license: String,
    pub license_url: String,
    pub license_sha256: String,
    pub license_size: u64,
    pub files: Vec<GoogleFile>,
}

const LICENSE_FILE: &str = "OFL.txt";

/// Bundled families with their upstream source (licence ids are in `assets/fonts/*.attribution`).
const BUNDLED_SOURCES: &[(&str, &str)] = &[
    ("Inter", "https://github.com/rsms/inter"),
    (
        "Noto Serif",
        "https://github.com/notofonts/notofonts.github.io",
    ),
    (
        "VibeCut Sans CJK",
        "https://github.com/notofonts/noto-cjk/releases/tag/Sans2.004",
    ),
];
const BUNDLED_LICENSE: &str = "OFL-1.1";

type Scanner = Box<dyn Fn() -> Vec<ScannedFace> + Send + Sync>;

/// The font catalog and download cache. One store serves a process; it holds no per-project state.
pub struct FontStore {
    root: PathBuf,
    google: Vec<GoogleFamily>,
    scan: Scanner,
    system: OnceLock<Vec<ScannedFace>>,
    transport: Arc<dyn DownloadTransport>,
    active: Arc<Mutex<BTreeSet<String>>>,
}

enum Cache {
    Missing,
    Invalid,
    /// Verified bytes of each font file, in catalog order.
    Valid(Vec<Vec<u8>>),
}

struct Active {
    set: Arc<Mutex<BTreeSet<String>>>,
    slug: String,
}

/// Private staging directory, removed on drop unless published.
struct Staging {
    path: PathBuf,
    published: bool,
}
mod catalog;

mod loading;

mod downloads;

pub use catalog::{curated_google_families, scan_directory, scan_system};

use loading::{read_cache, styles_of};

use downloads::{hex, sidecar};
