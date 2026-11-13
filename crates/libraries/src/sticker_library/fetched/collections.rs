//! The curated collection table: what each downloadable collection is, where it is pinned and
//! under which licence. Facts here were read from each collection's own licence file at the pinned
//! commit; the file table is [`super::entries`].
use super::entries::ENTRIES;
use super::sanitize::MAX_SVG_BYTES;

/// The Stickers panel's category tabs. `Shapes` is served by graphic definitions, never by a
/// collection; `Flags` is the bundled flag set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Emoji,
    Flags,
    Shapes,
    Icons,
}

impl Category {
    /// Every category, in tab order.
    pub const ALL: [Self; 4] = [Self::Emoji, Self::Flags, Self::Shapes, Self::Icons];

    /// Stable lowercase name.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Emoji => "emoji",
            Self::Flags => "flags",
            Self::Shapes => "shapes",
            Self::Icons => "icons",
        }
    }
}

/// One pinned file as written in the generated table.
pub(super) struct RawEntry {
    pub(super) collection: &'static str,
    pub(super) value: &'static str,
    pub(super) name: &'static str,
    pub(super) keywords: &'static [&'static str],
    pub(super) path: &'static str,
    pub(super) size: u64,
    pub(super) sha256: &'static str,
}

/// One pinned, downloadable file.
#[derive(Clone, Debug)]
pub struct Entry {
    /// The sticker id value: `<collection id>:<value>`.
    pub value: String,
    pub name: String,
    pub keywords: Vec<String>,
    /// Path inside the collection repository at the pinned commit.
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

/// A downloadable collection with its licence record and its pinned files.
#[derive(Clone, Debug)]
pub struct Collection {
    /// Provider id of its stickers and its directory name in the cache.
    pub id: String,
    pub name: String,
    pub category: Category,
    pub author: String,
    /// `owner/name` on the hosting site.
    pub repo: String,
    /// Full commit every URL and hash below belongs to.
    pub commit: String,
    /// SPDX id.
    pub license: String,
    /// Path of the licence text inside the repository at `commit`.
    pub license_path: String,
    pub license_size: u64,
    pub license_sha256: String,
    pub entries: Vec<Entry>,
}

/// Name of the licence text inside a cached collection directory.
pub(super) const LICENSE_FILE: &str = "LICENSE";

impl Collection {
    /// The collection's home page.
    pub fn source(&self) -> String {
        format!("https://github.com/{}", self.repo)
    }

    /// The pinned download URL of a repository path.
    pub fn raw_url(&self, path: &str) -> String {
        format!(
            "https://raw.githubusercontent.com/{}/{}/{path}",
            self.repo, self.commit
        )
    }

    /// Bytes one download transfers: every file plus the licence text.
    pub fn download_bytes(&self) -> u64 {
        self.entries
            .iter()
            .map(|entry| entry.size)
            .sum::<u64>()
            .saturating_add(self.license_size)
    }

    /// The entry for an id value.
    pub fn entry(&self, value: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.value == value)
    }

    /// Whether every field is well formed: ids and paths are plain, hashes are 64 hex digits,
    /// sizes are within the SVG limit and values are unique.
    pub(super) fn validate(&self) -> bool {
        let plain = |text: &str| {
            !text.is_empty()
                && text.len() <= 48
                && text
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        };
        let hex = |text: &str, len: usize| {
            text.len() == len
                && text
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        let path = |text: &str| {
            !text.is_empty()
                && !text.starts_with('/')
                && text
                    .split('/')
                    .all(|part| !part.is_empty() && part != "." && part != "..")
        };
        let mut values = std::collections::BTreeSet::new();
        plain(&self.id)
            && matches!(self.category, Category::Emoji | Category::Icons)
            && !self.name.is_empty()
            && !self.author.is_empty()
            && self.repo.split('/').count() == 2
            && hex(&self.commit, 40)
            && !self.license.is_empty()
            && path(&self.license_path)
            && hex(&self.license_sha256, 64)
            && self.license_size > 0
            && !self.entries.is_empty()
            && self.entries.iter().all(|entry| {
                plain(&entry.value)
                    && !entry.name.is_empty()
                    && path(&entry.path)
                    && hex(&entry.sha256, 64)
                    && entry.size > 0
                    && entry.size <= MAX_SVG_BYTES as u64
                    && values.insert(entry.value.as_str())
            })
    }
}

struct Spec {
    id: &'static str,
    name: &'static str,
    category: Category,
    author: &'static str,
    repo: &'static str,
    commit: &'static str,
    license: &'static str,
    license_path: &'static str,
    license_size: u64,
    license_sha256: &'static str,
}

/// Licence ids are those of the collection's own licence file at the pinned commit. Noto's images
/// are Apache-2.0 per `2D/svg/LICENSE`; the region-flag folder carries its own public-domain notice
/// and is not curated here (flags come from the bundled flag-icons set). Lucide's curated icons
/// avoid the names its licence file lists as Feather-derived, so only its ISC block applies.
const SPECS: [Spec; 3] = [
    Spec {
        id: "noto",
        name: "Noto Emoji",
        category: Category::Emoji,
        author: "Google",
        repo: "googlefonts/noto-emoji",
        commit: "99701e07d3d1fe1bdbcdb14964547f0fd592966f",
        license: "Apache-2.0",
        license_path: "2D/svg/LICENSE",
        license_size: 574,
        license_sha256: "611ceab36dae96644ca84e8ace6873821790192bf6f73b0d0624a21b24b4b332",
    },
    Spec {
        id: "lucide",
        name: "Lucide",
        category: Category::Icons,
        author: "Lucide Icons and Contributors",
        repo: "lucide-icons/lucide",
        commit: "64502ae56389321a922bd498709d4184b0e61543",
        license: "ISC",
        license_path: "LICENSE",
        license_size: 3208,
        license_sha256: "b495047bd93a9b06913511076f504daba17d5bbeb3e0650f3bb53a4220329c57",
    },
    Spec {
        id: "phosphor",
        name: "Phosphor Icons",
        category: Category::Icons,
        author: "Phosphor Icons",
        repo: "phosphor-icons/core",
        commit: "2b75f3ad12b420c9504ef05df8d2564a28f8500e",
        license: "MIT",
        license_path: "LICENSE",
        license_size: 1071,
        license_sha256: "b5b1f1da112d18ea2147decfd48ddc1bf2b5aeb6c265381579340e95b15a2bb2",
    },
];

/// The curated collections in table order: Noto Emoji, Lucide, Phosphor Icons.
pub fn curated_collections() -> Vec<Collection> {
    SPECS
        .iter()
        .map(|spec| Collection {
            id: spec.id.to_owned(),
            name: spec.name.to_owned(),
            category: spec.category,
            author: spec.author.to_owned(),
            repo: spec.repo.to_owned(),
            commit: spec.commit.to_owned(),
            license: spec.license.to_owned(),
            license_path: spec.license_path.to_owned(),
            license_size: spec.license_size,
            license_sha256: spec.license_sha256.to_owned(),
            entries: ENTRIES
                .iter()
                .filter(|raw| raw.collection == spec.id)
                .map(|raw| Entry {
                    value: raw.value.to_owned(),
                    name: raw.name.to_owned(),
                    keywords: raw.keywords.iter().map(|word| (*word).to_owned()).collect(),
                    path: raw.path.to_owned(),
                    size: raw.size,
                    sha256: raw.sha256.to_owned(),
                })
                .collect(),
        })
        .collect()
}
