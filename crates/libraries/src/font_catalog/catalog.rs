//! Discovers available font families.
use super::{
    Arc, BTreeMap, BTreeSet, BUNDLED_LICENSE, BUNDLED_SOURCES, Cache, DownloadTransport, EditError,
    FontEntry, FontError, FontOrigin, FontState, FontStore, GOOGLE_FONTS_COMMIT, GoogleFamily,
    GoogleFile, LICENSE_FILE, MAX_FAMILY_FACES, MAX_SCANNED_FACES, Mutex, OnceLock, Path, PathBuf,
    Result, ScannedFace, styles_of,
};
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

pub(super) fn collect(db: &fontdb::Database) -> Vec<ScannedFace> {
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

impl GoogleFamily {
    pub(super) fn validate(&self) -> bool {
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

    pub(super) fn download_bytes(&self) -> u64 {
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
}

impl FontStore {
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
}

impl FontStore {
    pub(super) fn system_faces(&self) -> &[ScannedFace] {
        self.system.get_or_init(|| (self.scan)())
    }
}

impl FontStore {
    pub(super) fn google_family(&self, family: &str) -> Option<&GoogleFamily> {
        self.google
            .iter()
            .find(|g| g.family.eq_ignore_ascii_case(family))
    }
}

impl FontStore {
    pub(super) fn system_family(&self, family: &str) -> Vec<&ScannedFace> {
        self.system_faces()
            .iter()
            .filter(|face| face.family.eq_ignore_ascii_case(family))
            .take(MAX_FAMILY_FACES)
            .collect()
    }
}

impl FontStore {
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
}
