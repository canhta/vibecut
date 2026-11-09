//! Persists the saved sound collection.
use super::{
    Error, LIBRARY_FORMAT, LIBRARY_VERSION, MAX_LIBRARY_BYTES, MAX_SAVED, MAX_TEXT, Mutex, Path,
    SavedSound, SearchPage, SearchRequest, SearchSource, Sound, SoundLibrary, SoundResult,
    StoredLibrary, commercial, fs, url,
};
impl SoundLibrary {
    pub fn open(root: &Path) -> Sound<Self> {
        private_directory(root)?;
        let root = fs::canonicalize(root)?;
        let audio = root.join("audio");
        private_directory(&audio)?;
        // Lease a stable catalog identity, separately from library.json, whose
        // format/publication is owned here rather than by project persistence.
        let lease = project::WriterLease::acquire(&root.join("sounds.vibecut"))?;
        let state = Mutex::new(load_library(&root)?);
        Ok(Self {
            root,
            audio,
            lease,
            state,
        })
    }
}

impl SoundLibrary {
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl SoundLibrary {
    pub fn saved(&self) -> Sound<Vec<SavedSound>> {
        Ok(self
            .state
            .lock()
            .map_err(|_| Error::LibraryCorrupt)?
            .clone())
    }
}

impl SoundLibrary {
    pub fn is_saved(&self, id: u64) -> Sound<bool> {
        Ok(self
            .state
            .lock()
            .map_err(|_| Error::LibraryCorrupt)?
            .iter()
            .any(|saved| saved.sound.id == id))
    }
}

impl SoundLibrary {
    /// Save is an idempotent upsert. A changed audio URL invalidates the old
    /// content-addressed preview when no other saved sound still cites it.
    pub fn save(&self, sound: &SoundResult) -> Sound<SavedSound> {
        validate_result(sound)?;
        let mut saved = SavedSound {
            sound: sound.clone(),
            saved_at_unix_ms: now_ms(),
        };
        let mut state = self.state.lock().map_err(|_| Error::LibraryCorrupt)?;
        let mut next = state.clone();
        if let Some(existing) = next.iter_mut().find(|item| item.sound.id == sound.id) {
            saved.saved_at_unix_ms = saved.saved_at_unix_ms.max(existing.saved_at_unix_ms);
            let old_url = existing.sound.audio_url.clone();
            *existing = saved.clone();
            self.persist(&next)?;
            *state = next;
            if old_url != sound.audio_url
                && !state.iter().any(|item| item.sound.audio_url == old_url)
            {
                let _ = self.remove_cached(&old_url);
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
                .then_with(|| a.sound.id.cmp(&b.sound.id))
        });
        self.persist(&next)?;
        *state = next;
        Ok(saved)
    }
}

impl SoundLibrary {
    pub fn unsave(&self, id: u64) -> Sound<bool> {
        let mut state = self.state.lock().map_err(|_| Error::LibraryCorrupt)?;
        let before = state.len();
        let removed_url: Option<String> = state
            .iter()
            .find(|item| item.sound.id == id)
            .map(|item| item.sound.audio_url.clone());
        let next: Vec<_> = state
            .iter()
            .filter(|item| item.sound.id != id)
            .cloned()
            .collect();
        let changed = next.len() != before;
        if changed {
            self.persist(&next)?;
            *state = next;
            if let Some(url) = removed_url
                && !state.iter().any(|item| item.sound.audio_url == url)
            {
                let _ = self.remove_cached(&url);
            }
        }
        Ok(changed)
    }
}

impl SoundLibrary {
    /// Clear saved sounds and every cached preview. The directory remains usable.
    pub fn clear_all(&self) -> Sound<usize> {
        let mut state = self.state.lock().map_err(|_| Error::LibraryCorrupt)?;
        let removed = state.len();
        self.persist(&[])?;
        state.clear();
        for entry in fs::read_dir(&self.audio)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let _ = fs::remove_file(entry.path());
            }
        }
        Ok(removed)
    }
}

impl SoundLibrary {
    pub fn search_local(&self, request: &SearchRequest) -> Sound<SearchPage> {
        request.validate()?;
        let needle = request.query.to_lowercase();
        let mut sounds: Vec<SoundResult> = self
            .saved()?
            .into_iter()
            .map(|saved| saved.sound)
            .filter(|sound| !request.commercial_only || commercial(&sound.licence))
            .filter(|sound| {
                needle.is_empty()
                    || sound.name.to_lowercase().contains(&needle)
                    || sound.author.to_lowercase().contains(&needle)
            })
            .collect();
        sounds.sort_by(|a, b| b.downloads.cmp(&a.downloads).then_with(|| a.id.cmp(&b.id)));
        let count = sounds.len() as u64;
        let start = usize::try_from((request.page - 1).saturating_mul(request.page_size))
            .map_err(|_| Error::Limit)?;
        let results: Vec<_> = sounds
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

impl SoundLibrary {
    pub(super) fn persist(&self, state: &[SavedSound]) -> Sound<()> {
        self.lease.validate()?;
        let library = StoredLibrary {
            format: LIBRARY_FORMAT.into(),
            version: LIBRARY_VERSION,
            sounds: state.to_vec(),
        };
        let bytes = serde_json::to_vec(&library).map_err(|_| Error::LibraryCorrupt)?;
        if bytes.len() as u64 > MAX_LIBRARY_BYTES {
            return Err(Error::Limit);
        }
        project::atomic_write(&self.root.join("library.json"), &bytes)?;
        Ok(())
    }
}

fn load_library(root: &Path) -> Sound<Vec<SavedSound>> {
    let path = root.join("library.json");
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    if !meta.file_type().is_file() || meta.len() > MAX_LIBRARY_BYTES {
        return Err(Error::LibraryCorrupt);
    }
    let bytes = fs::read(&path)?;
    let stored: StoredLibrary =
        serde_json::from_slice(&bytes).map_err(|_| Error::LibraryCorrupt)?;
    if stored.format != LIBRARY_FORMAT
        || stored.version != LIBRARY_VERSION
        || stored.sounds.len() > MAX_SAVED
    {
        return Err(Error::LibraryCorrupt);
    }
    for saved in &stored.sounds {
        validate_result(&saved.sound)?;
    }
    Ok(stored.sounds)
}
pub(super) fn validate_result(sound: &SoundResult) -> Sound<()> {
    if sound.id == 0
        || sound.name.is_empty()
        || sound.author.is_empty()
        || sound.licence.is_empty()
        || sound.attribution.is_empty()
        || sound.name.len() > MAX_TEXT
        || sound.author.len() > MAX_TEXT
        || sound.licence.len() > MAX_TEXT
        || sound.attribution.len() > MAX_TEXT
        || !url(&sound.source_url)
        || !url(&sound.audio_url)
        || sound.duration_ms == 0
        || sound.duration_ms > 86_400_000
    {
        return Err(Error::InvalidResponse);
    }
    Ok(())
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
