//! Persists the saved music collection.
use super::{
    AtomicBool, Deref, Error, LIBRARY_FORMAT, LIBRARY_VERSION, MAX_LIBRARY_BYTES, MAX_SAVED, Music,
    MusicLibrary, MusicResult, Mutex, Operation, OwnedDirectory, Path, Read, SavedMusic,
    SearchPage, SearchRequest, SearchSource, StoredLibrary, fs,
};
impl MusicLibrary {
    pub fn open(root: &Path) -> Music<Self> {
        let root = OwnedDirectory::open(root)?;
        // Lease a stable catalog identity, separately from library.json, whose
        // format/publication is owned here rather than by project persistence.
        let lease = project::WriterLease::acquire(&root.path().join("music.vibecut"))?;
        root.validate()?;
        lease.validate()?;
        let state = Mutex::new(load_library(root.path())?);
        let audio = root.child(std::ffi::OsStr::new("audio"))?;
        root.validate()?;
        lease.validate()?;
        Ok(Self {
            root,
            audio,
            lease,
            state,
            operation: AtomicBool::new(false),
        })
    }
}

impl MusicLibrary {
    /// Admitted path projection only; filesystem operations revalidate internally.
    pub fn root(&self) -> &Path {
        self.root.path()
    }
}

impl MusicLibrary {
    pub fn saved(&self) -> Music<Vec<SavedMusic>> {
        Ok(self
            .state
            .lock()
            .map_err(|_| Error::LibraryCorrupt)?
            .clone())
    }
}

impl MusicLibrary {
    pub fn is_saved(&self, id: &str) -> Music<bool> {
        Ok(self
            .state
            .lock()
            .map_err(|_| Error::LibraryCorrupt)?
            .iter()
            .any(|saved| saved.music.id == id))
    }
}

impl MusicLibrary {
    /// Admit provenance before an upsert (at most 512 tracks / 1 MiB JSON).
    /// Identical NFC records retain their time without a write; changed records
    /// publish before memory changes. Project admission failures retain their
    /// typed cause. A changed audio URL invalidates the unreferenced old cache.
    /// Cleanup errors occur after the changed catalog has committed.
    pub fn save(&self, music: &MusicResult) -> Music<SavedMusic> {
        let music = MusicResult::from(music.library_music()?);
        let mut saved = SavedMusic {
            music: music.clone(),
            saved_at_unix_ms: now_ms(),
        };
        let operation = self.operation()?;
        let mut next = self.saved()?;
        if let Some(existing) = next.iter_mut().find(|item| item.music.id == music.id) {
            if existing.music == music {
                return Ok(existing.clone());
            }
            saved.saved_at_unix_ms = saved.saved_at_unix_ms.max(existing.saved_at_unix_ms);
            let old_url = existing.music.audio_url.clone();
            *existing = saved.clone();
            let remove = old_url != music.audio_url
                && !next.iter().any(|item| item.music.audio_url == old_url);
            operation.persist(&next)?;
            operation.commit(next)?;
            if remove {
                operation.remove_cached(&old_url)?;
            }
            return Ok(saved);
        }
        if next.len() >= MAX_SAVED {
            return Err(Error::Limit);
        }
        next.push(saved.clone());
        next.sort_by(|a, b| {
            b.saved_at_unix_ms
                .cmp(&a.saved_at_unix_ms)
                .then_with(|| a.music.id.cmp(&b.music.id))
        });
        operation.persist(&next)?;
        operation.commit(next)?;
        Ok(saved)
    }
}

impl MusicLibrary {
    /// Unsave before unreferenced cache cleanup. Cleanup failure is typed after
    /// the catalog commits; inspect Saved before retrying a refused cleanup.
    pub fn unsave(&self, id: &str) -> Music<bool> {
        let operation = self.operation()?;
        let state = self.saved()?;
        let before = state.len();
        let removed_url: Option<String> = state
            .iter()
            .find(|item| item.music.id == id)
            .map(|item| item.music.audio_url.clone());
        let next: Vec<_> = state
            .iter()
            .filter(|item| item.music.id != id)
            .cloned()
            .collect();
        let changed = next.len() != before;
        if changed {
            let remove =
                removed_url.filter(|url| !next.iter().any(|item| item.music.audio_url == *url));
            operation.persist(&next)?;
            operation.commit(next)?;
            if let Some(url) = remove {
                operation.remove_cached(&url)?;
            }
        }
        Ok(changed)
    }
}

impl MusicLibrary {
    /// Clear Saved, then only canonical owned cache artifacts (at most 10,000
    /// enumerated entries). Preflight/publication failure preserves the catalog
    /// and cache. Cleanup errors are returned after the empty catalog commits;
    /// remaining files can be cleaned by retry. Unknown entries are preserved.
    pub fn clear_all(&self) -> Music<usize> {
        let operation = self.operation()?;
        let removed = self.saved()?.len();
        let paths = operation.cache_files()?;
        operation.persist(&[])?;
        operation.commit(Vec::new())?;
        for path in paths {
            operation.remove_path(&path)?;
        }
        Ok(removed)
    }
}

impl MusicLibrary {
    pub fn search_local(&self, request: &SearchRequest) -> Music<SearchPage> {
        request.validate()?;
        let needle = request.query.to_lowercase();
        let mut tracks: Vec<MusicResult> = self
            .saved()?
            .into_iter()
            .map(|saved| saved.music)
            .filter(|music| {
                needle.is_empty()
                    || music.title.to_lowercase().contains(&needle)
                    || music.author.to_lowercase().contains(&needle)
            })
            .collect();
        tracks.sort_by(|a, b| {
            b.duration_ms
                .cmp(&a.duration_ms)
                .then_with(|| a.id.cmp(&b.id))
        });
        let count = tracks.len() as u64;
        let start = usize::try_from((request.page - 1).saturating_mul(request.page_size))
            .map_err(|_| Error::Limit)?;
        let results: Vec<_> = tracks
            .into_iter()
            .skip(start)
            .take(request.page_size as usize)
            .collect();
        let has_next = start.saturating_add(results.len()) < count as usize;
        Ok(SearchPage {
            count,
            page: request.page,
            page_size: request.page_size,
            results,
            has_next,
            source: SearchSource::Local,
        })
    }
}

impl<T: Deref<Target = MusicLibrary>> Operation<T> {
    fn commit(&self, state: Vec<SavedMusic>) -> Music<()> {
        *self.0.state.lock().map_err(|_| Error::LibraryCorrupt)? = state;
        Ok(())
    }
    fn persist(&self, state: &[SavedMusic]) -> Music<()> {
        self.0.validate()?;
        let library = StoredLibrary {
            format: LIBRARY_FORMAT.into(),
            version: LIBRARY_VERSION,
            tracks: state.to_vec(),
        };
        let bytes = serde_json::to_vec(&library).map_err(|_| Error::LibraryCorrupt)?;
        if bytes.len() as u64 > MAX_LIBRARY_BYTES {
            return Err(Error::Limit);
        }
        project::atomic::stage_validated(
            &self.0.root().join("library.json"),
            |file| {
                self.0.validate()?;
                std::io::Write::write_all(file, &bytes)?;
                Ok(())
            },
            |_| self.0.validate(),
        )?;
        Ok(())
    }
}

fn load_library(root: &Path) -> Music<Vec<SavedMusic>> {
    let path = root.join("library.json");
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    if !meta.file_type().is_file() || meta.len() > MAX_LIBRARY_BYTES {
        return Err(Error::LibraryCorrupt);
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)?
        .take(MAX_LIBRARY_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_LIBRARY_BYTES {
        return Err(Error::LibraryCorrupt);
    }
    // Inspect only the envelope before interpreting the current semantic body.
    // Incompatible cache versions drop; there is no older-format reader.
    #[derive(serde::Deserialize)]
    struct Header {
        format: String,
        version: u32,
    }
    let header: Header = serde_json::from_slice(&bytes).map_err(|_| Error::LibraryCorrupt)?;
    if header.format != LIBRARY_FORMAT {
        return Err(Error::LibraryCorrupt);
    }
    if header.version != LIBRARY_VERSION {
        return Ok(Vec::new());
    }
    let mut stored: StoredLibrary =
        serde_json::from_slice(&bytes).map_err(|_| Error::LibraryCorrupt)?;
    if stored.tracks.len() > MAX_SAVED {
        return Err(Error::LibraryCorrupt);
    }
    for saved in &mut stored.tracks {
        saved.music = saved.music.library_music()?.into();
    }
    Ok(stored.tracks)
}
pub(super) fn validate_result(music: &MusicResult) -> Music<()> {
    music.library_music().map(|_| ())
}
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}
pub(super) fn private_directory(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}
