//! Font catalog: bundled, system and curated Google families, lazy loading into `text`, and an
//! explicit, hash-pinned download cache. Nothing here touches the network except
//! [`FontStore::submit_download`]; rendering only ever reads files already verified on disk.
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

/// Enumerate font files below `dir` (recursively) with `fontdb`; malformed files are skipped.
pub fn scan_directory(dir: &Path) -> Vec<ScannedFace> {
    let mut db = fontdb::Database::new();
    db.load_fonts_dir(dir);
    collect(&db)
}

/// Enumerate the operating system's installed fonts with `fontdb`.
pub fn scan_system() -> Vec<ScannedFace> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    collect(&db)
}

fn collect(db: &fontdb::Database) -> Vec<ScannedFace> {
    let mut faces: Vec<ScannedFace> = db
        .faces()
        .filter_map(|face| {
            let fontdb::Source::File(path) = &face.source else {
                return None;
            };
            let (family, _) = face.families.first()?;
            Some(ScannedFace {
                family: family.clone(),
                weight: face.weight.0.clamp(1, 1000),
                italic: face.style != fontdb::Style::Normal,
                path: path.clone(),
                index: face.index,
            })
        })
        .collect();
    faces.sort();
    faces.truncate(MAX_SCANNED_FACES);
    faces
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

impl GoogleFamily {
    fn validate(&self) -> bool {
        let plain = |name: &str| {
            !name.is_empty()
                && name.len() <= 128
                && name != "."
                && name != ".."
                && !name.contains(['/', '\\', ':'])
        };
        let hex = |value: &str| value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit());
        let https = |url: &str| url.starts_with("https://") && url.len() <= 2048;
        !self.family.trim().is_empty()
            && self.family.len() <= 256
            && !self.slug.is_empty()
            && self
                .slug
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !self.files.is_empty()
            && self.files.len() <= MAX_FAMILY_FACES
            && https(&self.source)
            && https(&self.license_url)
            && hex(&self.license_sha256)
            && self.license_size <= 1 << 20
            && self.files.iter().all(|f| {
                plain(&f.name)
                    && f.name != LICENSE_FILE
                    && !f.name.ends_with(".attribution")
                    && https(&f.url)
                    && hex(&f.sha256)
                    && f.size > 0
                    && f.size <= text::MAX_FONT_BYTES as u64
                    && (1..=1000).contains(&f.weight)
            })
            && self
                .files
                .iter()
                .map(|f| f.name.as_str())
                .collect::<BTreeSet<_>>()
                .len()
                == self.files.len()
    }

    fn download_bytes(&self) -> u64 {
        self.files
            .iter()
            .fold(self.license_size, |sum, f| sum.saturating_add(f.size))
    }
}

/// The curated downloadable families (all OFL-1.1, with Vietnamese coverage except Bebas
/// Neue), pinned to one commit of the upstream repository.
pub fn curated_google_families() -> Vec<GoogleFamily> {
    // (family, directory, author, license sha256, license size,
    //  [(file, weight, italic, sha256, size)])
    type Row = (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        u64,
        &'static [(&'static str, u16, bool, &'static str, u64)],
    );
    const ROWS: &[Row] = &[
        (
            "Anton",
            "anton",
            "Vernon Adams",
            "ee67e6ee22790b7929f1a3769ca2801d565c64b5a9096942c1adf5596de9c9e4",
            4484,
            &[(
                "Anton-Regular.ttf",
                400,
                false,
                "a4ba3a92350ebb031da0cb47630ac49eb265082ca1bc0450442f4a83ab947cab",
                170_812,
            )],
        ),
        (
            "Bebas Neue",
            "bebasneue",
            "Ryoichi Tsunekawa",
            "72082f6cb4d04be2ecf7cc7d9e1e7d73787f0af8a5a278a47cade70c16b78341",
            4337,
            &[(
                "BebasNeue-Regular.ttf",
                400,
                false,
                "08e4623805102d819f58601e46e345648846075e363b2ceb23313c2d1c83ec73",
                61_400,
            )],
        ),
        (
            "Be Vietnam Pro",
            "bevietnampro",
            "Lâm Bảo, Tony Le, ViệtAnh Nguyễn",
            "6b7f8f73609a25ea78c891e34cf37b06f8a676b7ea986e941e43b009110f2a85",
            4397,
            &[
                (
                    "BeVietnamPro-Regular.ttf",
                    400,
                    false,
                    "cd1ef6e9d7db28ad5cdb88a65ccbe693870e60d340b791f349d248342b4fe4c3",
                    132_948,
                ),
                (
                    "BeVietnamPro-Bold.ttf",
                    700,
                    false,
                    "7f738fe5c43c8872807b20e2d30d42163618de8a4daf7f48a939adac32c16847",
                    140_300,
                ),
            ],
        ),
        (
            "Lobster",
            "lobster",
            "Impallari Type",
            "88aece7d90f2bb7049719f11619a560af22af3451af141f12aa4f46bb157a99b",
            4430,
            &[(
                "Lobster-Regular.ttf",
                400,
                false,
                "d6568e697fd50cedc0be04d8aae4127fe95add607e7bff954ca88604be80c205",
                406_076,
            )],
        ),
        (
            "Pacifico",
            "pacifico",
            "Vernon Adams, Jacques Le Bailly, Botjo Nikoltchev, Ani Petrova",
            "a47e5daeda73568969395c656823102678f2eefb0d7d7ecb47aac4cc17e42204",
            4389,
            &[(
                "Pacifico-Regular.ttf",
                400,
                false,
                "5b6c0d5334a7bf77dea52b975c5a0c408878c0f7115ed5b6fb151f634b7bf701",
                329_380,
            )],
        ),
        (
            "Patrick Hand",
            "patrickhand",
            "Patrick Wagesreiter",
            "377f4f9c19e935228552478eb68cc2ed82910988a60ba60e2ac73b09f32d02d1",
            4376,
            &[(
                "PatrickHand-Regular.ttf",
                400,
                false,
                "0f173b3e6cb6d1af25babf7f0057c5ac4ee11f9992b0469bb817e967ef4ad0fc",
                214_772,
            )],
        ),
    ];
    let raw = |dir: &str, file: &str| {
        format!(
            "https://raw.githubusercontent.com/google/fonts/{GOOGLE_FONTS_COMMIT}/ofl/{dir}/{file}"
        )
    };
    ROWS.iter()
        .map(
            |(family, dir, author, license_sha, license_size, files)| GoogleFamily {
                family: (*family).to_owned(),
                slug: (*dir).to_owned(),
                author: (*author).to_owned(),
                source: format!(
                    "https://github.com/google/fonts/tree/{GOOGLE_FONTS_COMMIT}/ofl/{dir}"
                ),
                license: "OFL-1.1".to_owned(),
                license_url: raw(dir, LICENSE_FILE),
                license_sha256: (*license_sha).to_owned(),
                license_size: *license_size,
                files: files
                    .iter()
                    .map(|(name, weight, italic, sha, size)| GoogleFile {
                        name: (*name).to_owned(),
                        url: raw(dir, name),
                        sha256: (*sha).to_owned(),
                        size: *size,
                        weight: *weight,
                        italic: *italic,
                    })
                    .collect(),
            },
        )
        .collect()
}

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

impl FontStore {
    /// A store caching downloads under `root`. `scan` enumerates installed fonts and runs at most
    /// once, on first use.
    ///
    /// # Errors
    ///
    /// Refuses a catalog entry with an unsafe slug or file name, a non-HTTPS URL or a malformed
    /// hash.
    pub fn new(
        root: PathBuf,
        google: Vec<GoogleFamily>,
        scan: impl Fn() -> Vec<ScannedFace> + Send + Sync + 'static,
        transport: Arc<dyn DownloadTransport>,
    ) -> Result<Self> {
        if !google.iter().all(GoogleFamily::validate)
            || google
                .iter()
                .map(|g| g.family.to_lowercase())
                .collect::<BTreeSet<_>>()
                .len()
                != google.len()
            || google
                .iter()
                .map(|g| g.slug.as_str())
                .collect::<BTreeSet<_>>()
                .len()
                != google.len()
        {
            return Err(FontError::Catalog.into());
        }
        Ok(Self {
            root,
            google,
            scan: Box::new(scan),
            system: OnceLock::new(),
            transport,
            active: Arc::new(Mutex::new(BTreeSet::new())),
        })
    }

    /// The application composition: OS fonts, the curated catalog, the shared HTTPS transport
    /// and the per-user cache `<cache>/VibeCut/fonts`. Nothing is read or fetched until used.
    ///
    /// # Errors
    ///
    /// Fails when the OS cache location is unavailable.
    pub fn native() -> Result<Self> {
        let root = ai::models::cache_dir()?
            .parent()
            .map(|parent| parent.join("fonts"))
            .ok_or(EditError::Invalid("font cache location"))?;
        Self::new(
            root,
            curated_google_families(),
            scan_system,
            Arc::new(ai::transport::UreqTransport::new()),
        )
    }

    fn system_faces(&self) -> &[ScannedFace] {
        self.system.get_or_init(|| (self.scan)())
    }

    fn google_family(&self, family: &str) -> Option<&GoogleFamily> {
        self.google
            .iter()
            .find(|g| g.family.eq_ignore_ascii_case(family))
    }

    fn system_family(&self, family: &str) -> Vec<&ScannedFace> {
        self.system_faces()
            .iter()
            .filter(|face| face.family.eq_ignore_ascii_case(family))
            .take(MAX_FAMILY_FACES)
            .collect()
    }

    /// Families whose name contains `query` (case-insensitive), prefix matches first and then
    /// alphabetical. One entry per family name: bundled wins over a cached download, which wins
    /// over a system install, which wins over a download that is not cached yet.
    ///
    /// # Errors
    ///
    /// Fails on cache IO other than a missing entry.
    pub fn list(&self, query: Option<&str>) -> Result<Vec<FontEntry>> {
        let needle = query
            .map(|q| q.trim().to_lowercase())
            .filter(|q| !q.is_empty());
        let mut best: BTreeMap<String, (u8, FontEntry)> = BTreeMap::new();
        let mut offer = |rank: u8, entry: FontEntry| {
            let key = entry.family.to_lowercase();
            if best.get(&key).is_none_or(|(old, _)| rank < *old) {
                best.insert(key, (rank, entry));
            }
        };
        for (family, styles) in text::families() {
            let source = BUNDLED_SOURCES
                .iter()
                .find(|(name, _)| *name == family)
                .map(|(_, source)| (*source).to_owned());
            offer(
                0,
                FontEntry {
                    family,
                    origin: FontOrigin::Bundled,
                    styles,
                    state: FontState::Available,
                    license: Some(BUNDLED_LICENSE.to_owned()),
                    source,
                    download_bytes: None,
                },
            );
        }
        for g in &self.google {
            let cached = matches!(self.cache(g)?, Cache::Valid(_));
            offer(
                if cached { 1 } else { 3 },
                FontEntry {
                    family: g.family.clone(),
                    origin: FontOrigin::Google,
                    styles: styles_of(g.files.iter().map(|f| (f.weight, f.italic))),
                    state: if cached {
                        FontState::Available
                    } else {
                        FontState::NotDownloaded
                    },
                    license: Some(g.license.clone()),
                    source: Some(g.source.clone()),
                    download_bytes: (!cached).then(|| g.download_bytes()),
                },
            );
        }
        let mut system: BTreeMap<&str, Vec<(u16, bool)>> = BTreeMap::new();
        for face in self.system_faces() {
            system
                .entry(face.family.as_str())
                .or_default()
                .push((face.weight, face.italic));
        }
        for (family, faces) in system {
            offer(
                2,
                FontEntry {
                    family: family.to_owned(),
                    origin: FontOrigin::System,
                    styles: styles_of(faces.into_iter()),
                    state: FontState::Available,
                    license: None,
                    source: None,
                    download_bytes: None,
                },
            );
        }
        let mut entries: Vec<FontEntry> = best
            .into_iter()
            .filter(|(key, _)| needle.as_ref().is_none_or(|n| key.contains(n.as_str())))
            .map(|(_, (_, entry))| entry)
            .collect();
        entries.sort_by_cached_key(|entry| {
            let key = entry.family.to_lowercase();
            (
                !needle.as_ref().is_some_and(|n| key.starts_with(n.as_str())),
                key,
                entry.family.clone(),
            )
        });
        Ok(entries)
    }

    /// Register the family's faces with `text` so layout resolves it, reading font files lazily
    /// and only now. Network is never used. Unknown and not-downloaded families are load
    /// states, not errors: layout then falls back to the default family deterministically.
    ///
    /// # Errors
    ///
    /// Fails on file IO or when `text` refuses every face of the family.
    pub fn load(&self, family: &str) -> Result<FontLoad> {
        let family = family.trim();
        if family.is_empty() || family.len() > 256 {
            return Err(EditError::Invalid("font family"));
        }
        let state = |state, origin, effective: &str, styles| FontLoad {
            family: family.to_owned(),
            state,
            origin,
            effective_family: effective.to_owned(),
            styles,
        };
        if let Some((name, styles)) = text::families()
            .into_iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(family))
        {
            return Ok(state(
                FontLoadState::Loaded,
                Some(FontOrigin::Bundled),
                &name,
                styles,
            ));
        }
        let mut blocked = None;
        if let Some(g) = self.google_family(family) {
            match self.cache(g)? {
                Cache::Valid(files) => {
                    let mut faces = Vec::new();
                    for (file, bytes) in g.files.iter().zip(files) {
                        faces.push(text::FontData {
                            family: g.family.clone(),
                            style: text::style_name(file.weight, file.italic),
                            weight: file.weight,
                            italic: file.italic,
                            index: 0,
                            bytes,
                        });
                    }
                    let styles = register(faces)?;
                    return Ok(state(
                        FontLoadState::Loaded,
                        Some(FontOrigin::Google),
                        &g.family,
                        styles,
                    ));
                }
                Cache::Invalid => blocked = Some(FontLoadState::CacheInvalid),
                Cache::Missing => blocked = Some(FontLoadState::NotDownloaded),
            }
        }
        let installed = self.system_family(family);
        if let Some(first) = installed.first() {
            let mut faces = Vec::new();
            for face in &installed {
                let Some(bytes) = read_font(&face.path)? else {
                    continue;
                };
                faces.push(text::FontData {
                    family: first.family.clone(),
                    style: text::style_name(face.weight, face.italic),
                    weight: face.weight,
                    italic: face.italic,
                    index: face.index,
                    bytes,
                });
            }
            let styles = register(faces)?;
            return Ok(state(
                FontLoadState::Loaded,
                Some(FontOrigin::System),
                &first.family,
                styles,
            ));
        }
        Ok(state(
            blocked.unwrap_or(FontLoadState::Unknown),
            blocked.map(|_| FontOrigin::Google),
            text::DEFAULT_FAMILY,
            Vec::new(),
        ))
    }

    fn cache(&self, g: &GoogleFamily) -> Result<Cache> {
        read_cache(&self.root, g)
    }

    /// Start the explicit download of a catalog family. `accepted` is the user's acknowledgement
    /// of the displayed source and licence; without it nothing starts. Files go to a private
    /// staging directory, are verified against the pinned size and SHA-256, and the finished
    /// directory is renamed into place once; a failed or cancelled job leaves no cache entry.
    ///
    /// # Errors
    ///
    /// `ConsentRequired` without acknowledgement; [`FontError::NotDownloadable`] for a family
    /// outside the catalog; [`FontError::DownloadActive`] while the same family is downloading;
    /// runner admission errors.
    pub fn submit_download(
        &self,
        runner: &mut JobRunner,
        family: &str,
        accepted: bool,
    ) -> Result<(JobId, CancelToken)> {
        if !accepted {
            return Err(EditError::ConsentRequired);
        }
        let g = self
            .google_family(family.trim())
            .ok_or(FontError::NotDownloadable)?
            .clone();
        let guard = Active::acquire(&self.active, &g.slug)?;
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::FontDownload,
            source: [0; 32],
            provider: "fonts",
            model: &g.slug,
            version: GOOGLE_FONTS_COMMIT,
            settings: &[],
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        let root = self.root.clone();
        let transport = self.transport.clone();
        runner.submit(
            JobSpec {
                kind: JobKind::DownloadFont,
                class: ResourceClass::Network,
                input: key,
                bytes: JobBytes {
                    compositor: WORK_BYTES,
                    ..Default::default()
                },
            },
            move |ctx| {
                let _hold = &guard;
                download(&root, &g, transport.as_ref(), ctx)?;
                Ok(Vec::new())
            },
        )
    }
}

/// Verified cache state of a catalog family: missing, tampered/stale, or the verified bytes.
fn read_cache(root: &Path, g: &GoogleFamily) -> Result<Cache> {
    let dir = root.join(&g.slug);
    match fs::symlink_metadata(&dir) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Ok(Cache::Invalid),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Cache::Missing),
        Err(error) => return Err(error.into()),
    }
    let mut files = Vec::new();
    let expected = g
        .files
        .iter()
        .map(|f| (f.name.as_str(), f.size, f.sha256.as_str()))
        .chain(std::iter::once((
            LICENSE_FILE,
            g.license_size,
            g.license_sha256.as_str(),
        )));
    for (name, size, sha256) in expected {
        let path = dir.join(name);
        let Some(bytes) = read_verified(&path, size, sha256)? else {
            return Ok(Cache::Invalid);
        };
        if !g.files.iter().any(|f| f.name == name) {
            continue;
        }
        if !fs::symlink_metadata(sidecar(&path)).is_ok_and(|m| m.is_file()) {
            return Ok(Cache::Invalid);
        }
        files.push(bytes);
    }
    Ok(Cache::Valid(files))
}

struct Active {
    set: Arc<Mutex<BTreeSet<String>>>,
    slug: String,
}
impl Active {
    fn acquire(set: &Arc<Mutex<BTreeSet<String>>>, slug: &str) -> Result<Self> {
        let mut held = set.lock().map_err(|_| EditError::JobPoisoned)?;
        if !held.insert(slug.to_owned()) {
            return Err(FontError::DownloadActive.into());
        }
        Ok(Self {
            set: set.clone(),
            slug: slug.to_owned(),
        })
    }
}
impl Drop for Active {
    fn drop(&mut self) {
        if let Ok(mut held) = self.set.lock() {
            held.remove(&self.slug);
        }
    }
}

fn styles_of(faces: impl Iterator<Item = (u16, bool)>) -> Vec<String> {
    let mut faces: Vec<(u16, bool)> = faces.collect();
    faces.sort_unstable();
    faces.dedup();
    faces
        .into_iter()
        .map(|(weight, italic)| text::style_name(weight, italic))
        .collect()
}

/// Register faces, tolerating a name that is already held by a different file. Returns the style
/// names now resolvable; fails only when no face could be registered.
fn register(faces: Vec<text::FontData>) -> Result<Vec<String>> {
    let mut styles: Vec<String> = Vec::new();
    let mut last = None;
    let mut registered = false;
    for face in faces {
        let style = face.style.clone();
        match text::register(face) {
            Ok(_) | Err(text::FontError::Conflict) => {
                registered = true;
                if !styles.contains(&style) {
                    styles.push(style);
                }
            }
            Err(error) => last = Some(error),
        }
    }
    match (registered, last) {
        (false, Some(error)) => Err(FontError::Face(error).into()),
        (false, None) => Err(EditError::Invalid("font family has no readable face")),
        _ => Ok(styles),
    }
}

/// A font file read for registration; `None` when it is too large or vanished since the scan.
fn read_font(path: &Path) -> Result<Option<Vec<u8>>> {
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !meta.is_file() || meta.len() > text::MAX_FONT_BYTES as u64 {
        return Ok(None);
    }
    Ok(Some(fs::read(path)?))
}

/// The file's bytes when it is a regular file of exactly `size` bytes with this SHA-256.
fn read_verified(path: &Path, size: u64, sha256: &str) -> Result<Option<Vec<u8>>> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !meta.is_file() || meta.len() != size {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    Ok((bytes.len() as u64 == size && hex(&Sha256::digest(&bytes)) == sha256).then_some(bytes))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(64), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn sidecar(font: &Path) -> PathBuf {
    let mut path = font.as_os_str().to_os_string();
    path.push(".attribution");
    PathBuf::from(path)
}

/// Private staging directory, removed on drop unless published.
struct Staging {
    path: PathBuf,
    published: bool,
}
impl Staging {
    fn create(root: &Path, slug: &str) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        fs::create_dir_all(root)?;
        let path = root.join(format!(
            ".partial-{slug}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self {
            path,
            published: false,
        })
    }
    fn publish(mut self, target: &Path) -> Result<()> {
        match fs::symlink_metadata(target) {
            Ok(meta) if meta.is_dir() => fs::remove_dir_all(target)?,
            Ok(_) => fs::remove_file(target)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        fs::rename(&self.path, target)?;
        self.published = true;
        Ok(())
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn download(
    root: &Path,
    g: &GoogleFamily,
    http: &dyn DownloadTransport,
    ctx: &JobContext,
) -> Result<()> {
    ctx.check_cancelled()?;
    let target = root.join(&g.slug);
    if matches!(read_cache(root, g)?, Cache::Valid(_)) {
        return Ok(());
    }
    let staging = Staging::create(root, &g.slug)?;
    let total = g.download_bytes().max(1);
    let mut done = 0u64;
    ctx.progress(JobStage::Download, 0.0)?;
    let items = g
        .files
        .iter()
        .map(|f| (f.name.as_str(), f.url.as_str(), f.size, f.sha256.as_str()))
        .chain(std::iter::once((
            LICENSE_FILE,
            g.license_url.as_str(),
            g.license_size,
            g.license_sha256.as_str(),
        )));
    for (name, url, size, sha256) in items {
        fetch(
            http,
            url,
            size,
            sha256,
            &staging.path.join(name),
            ctx,
            &mut done,
            total,
        )?;
    }
    for file in &g.files {
        let record = format!(
            "asset: {name}\ntitle: {family} ({name})\nauthor: {author}\nsource: {url}\nlicense: {license}\nlicense-file: {LICENSE_FILE}\nadded: {ADMITTED_BY}\nnotes: Downloaded on explicit user request from {page}; SHA-256 {sha}. Verified against the catalog pin before use.\n",
            name = file.name,
            family = g.family,
            author = g.author,
            url = file.url,
            license = g.license,
            page = g.source,
            sha = file.sha256,
        );
        fs::write(sidecar(&staging.path.join(&file.name)), record)?;
    }
    ctx.progress(JobStage::Verify, 1.0)?;
    ctx.commit_publication(|| staging.publish(&target))
}

#[allow(clippy::too_many_arguments)]
fn fetch(
    http: &dyn DownloadTransport,
    url: &str,
    size: u64,
    sha256: &str,
    path: &Path,
    ctx: &JobContext,
    done: &mut u64,
    total: u64,
) -> Result<()> {
    let headers = [("Accept-Encoding".to_owned(), "identity".to_owned())];
    let reply = crate::models::response(http, url, &headers, ctx)?;
    if reply.status != 200 {
        return Err(FontError::Response.into());
    }
    let mut file = fs::File::create_new(path)?;
    let mut body = reply.body.take(size.saturating_add(1));
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; BUFFER];
    let mut written = 0u64;
    loop {
        ctx.check_cancelled()?;
        let read = body
            .read(&mut buffer)
            .map_err(|_| EditError::NetworkTransient)?;
        if read == 0 {
            break;
        }
        let chunk = buffer.get(..read).ok_or(EditError::Limit)?;
        written += read as u64;
        if written > size {
            return Err(FontError::Integrity.into());
        }
        hasher.update(chunk);
        file.write_all(chunk)?;
        *done += read as u64;
        ctx.progress(JobStage::Download, (*done as f32 / total as f32).min(1.0))?;
    }
    if written != size || hex(&hasher.finalize()) != sha256 {
        return Err(FontError::Integrity.into());
    }
    file.sync_all()?;
    Ok(())
}
