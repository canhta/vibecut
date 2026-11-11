//! Resolves font assets for project rendering.
use ai::transport::DownloadTransport;
use engine::cache::{ArtifactKind, CacheKey, KeyInputs};
use engine::jobs::ResourceClass;
use engine::jobs::{
    CancelToken, JobBytes, JobContext, JobId, JobKind, JobRunner, JobSpec, JobStage,
};
use engine::{Error as EngineError, Result};
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

use engine::FontError;
use engine::control::{FontEntry, FontLoad, FontLoadState, FontOrigin, FontState};

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
    file: Arc<dyn FontFile>,
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
mod formats;
pub use imports::{FontFile, LocalFontFile};
mod imports;

pub use catalog::{curated_google_families, scan_directory, scan_system};

use loading::{read_cache, styles_of};

use downloads::{hex, sidecar};
