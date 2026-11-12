//! Reuses verified music bytes.
use super::{
    AssetData, CachedMusic, ContentHash, Deref, Error, MAX_AUDIO_BYTES, MAX_CACHE_ENTRIES, Music,
    MusicLibrary, Operation, Path, PathBuf, Read, Result, SourceAsset, SourceProbe, Tick, Write,
    fs, url,
};
impl MusicLibrary {
    /// Path projection only; reading or mutating the cache revalidates ownership.
    pub fn cache_path(&self, url: &str) -> PathBuf {
        self.audio.path().join(format!("{}.audio", cache_key(url)))
    }
}

impl MusicLibrary {
    pub fn cached(&self, url: &str) -> Music<Option<CachedMusic>> {
        let operation = self.operation()?;
        let path = self.cache_path(url);
        operation.admit_target(&path)?;
        let Some(meta) = operation.cache_file(&path)? else {
            return Ok(None);
        };
        if meta.len() == 0 {
            return Err(Error::LibraryCorrupt);
        }
        if meta.len() > MAX_AUDIO_BYTES {
            return Err(Error::Limit);
        }
        let cached = cached_from_path(path, Some(url))?;
        self.validate()?;
        Ok(Some(cached))
    }
}

impl MusicLibrary {
    /// Store already downloaded bytes. Only complete, probed audio is published;
    /// a failed probe leaves no cache entry.
    pub fn store_cached(&self, source: &str, bytes: &[u8]) -> Music<CachedMusic> {
        if !url(source) || bytes.is_empty() || bytes.len() as u64 > MAX_AUDIO_BYTES {
            return Err(Error::Invalid("cached audio"));
        }
        let operation = self.operation()?;
        let path = self.cache_path(source);
        operation.admit_target(&path)?;
        project::atomic::stage_validated(
            &path,
            |file| {
                self.validate()?;
                file.write_all(bytes)?;
                Ok(())
            },
            |staged| {
                self.validate()?;
                let cached = cached_from_path(staged.to_owned(), Some(source))?;
                if cached.probe.duration.is_none_or(|d| d <= Tick::ZERO)
                    || !cached
                        .probe
                        .streams
                        .iter()
                        .any(|s| s.kind == media::StreamKind::Audio)
                {
                    return Err(Error::InvalidResponse);
                }
                operation.admit_target(&path)?;
                Ok(CachedMusic {
                    path: path.clone(),
                    ..cached
                })
            },
        )
    }
}

impl<T: Deref<Target = MusicLibrary>> Operation<T> {
    fn cache_file(&self, path: &Path) -> Music<Option<fs::Metadata>> {
        self.0.validate()?;
        if path.parent() != Some(self.0.audio.path())
            || !path.file_name().is_some_and(artifact_name)
        {
            return Err(Error::Invalid("cache path"));
        }
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_file()
            && !metadata.file_type().is_symlink()
            && fs::canonicalize(path)? == path
        {
            Ok(Some(metadata))
        } else {
            Ok(None)
        }
    }
    fn admit_target(&self, path: &Path) -> Music<()> {
        self.0.validate()?;
        match fs::symlink_metadata(path) {
            Ok(_) if self.cache_file(path)?.is_none() => Err(Error::LibraryCorrupt),
            Ok(_) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
    pub(super) fn cache_files(&self) -> Music<Vec<PathBuf>> {
        self.0.validate()?;
        let mut paths = Vec::new();
        let mut count = 0usize;
        for entry in fs::read_dir(self.0.audio.path())? {
            let entry = entry?;
            count = count.checked_add(1).ok_or(Error::Limit)?;
            if count > MAX_CACHE_ENTRIES {
                return Err(Error::Limit);
            }
            if artifact_name(&entry.file_name()) && self.cache_file(&entry.path())?.is_some() {
                paths.try_reserve(1).map_err(|_| Error::Limit)?;
                paths.push(entry.path());
            }
        }
        self.0.validate()?;
        Ok(paths)
    }
    pub(super) fn remove_path(&self, path: &Path) -> Music<()> {
        if self.cache_file(path)?.is_none() {
            return Ok(());
        }
        self.0.validate()?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
    pub(super) fn remove_cached(&self, url: &str) -> Music<()> {
        self.remove_path(&self.0.cache_path(url))
    }
}
fn artifact_name(name: &std::ffi::OsStr) -> bool {
    name.as_encoded_bytes()
        .strip_suffix(b".audio")
        .is_some_and(|hash| {
            hash.len() == 64
                && hash
                    .iter()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        })
}
fn cache_key(url: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"vibecut-music-v1");
    hasher.update(url.as_bytes());
    hasher.finalize().to_hex().to_string()
}
impl CachedMusic {
    pub fn duration(&self) -> Option<Tick> {
        self.probe
            .duration
            .filter(|duration| *duration > Tick::ZERO)
    }
    pub fn asset(&self, id: media::AssetId) -> Result<SourceAsset> {
        if !self
            .probe
            .streams
            .iter()
            .any(|s| s.kind == media::StreamKind::Audio)
        {
            return Err(Error::InvalidResponse.into());
        }
        let asset = SourceAsset::try_from(AssetData {
            ephemeral: false,
            id,
            path: self.path.clone(),
            content_hash: self.content_hash,
            probe: self.probe.clone(),
        })?;
        Ok(asset)
    }
    pub fn bytes(&self) -> Music<Vec<u8>> {
        let meta = fs::metadata(&self.path)?;
        if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_AUDIO_BYTES {
            return Err(Error::Limit);
        }
        let mut bytes = Vec::new();
        fs::File::open(&self.path)?
            .take(MAX_AUDIO_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.is_empty() || bytes.len() as u64 > MAX_AUDIO_BYTES {
            return Err(Error::Limit);
        }
        Ok(bytes)
    }
}
pub(super) fn cached_from_path(path: PathBuf, source: Option<&str>) -> Music<CachedMusic> {
    if let Some(source) = source
        && !url(source)
    {
        return Err(Error::Invalid("cached audio URL"));
    }
    let hash = ContentHash::read_with_cancel(&path, || false, |_, _| {})?;
    let probe = media::probe(&path)?;
    Ok(CachedMusic {
        path,
        content_hash: hash,
        probe: SourceProbe::from(&probe),
    })
}
impl<T: Deref<Target = MusicLibrary>> Operation<T> {
    pub(super) fn publish_temp(
        &self,
        source: &str,
        temp: &Path,
        cancelled: &dyn Fn() -> bool,
        fence: Option<&engine::jobs::JobContext>,
    ) -> Music<CachedMusic> {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        self.0.validate()?;
        if temp.parent() != Some(self.0.audio.path()) {
            return Err(Error::Invalid("cache staging path"));
        }
        let cached = cached_from_path(temp.to_owned(), Some(source))?;
        if cached.probe.duration.is_none_or(|d| d <= Tick::ZERO)
            || !cached
                .probe
                .streams
                .iter()
                .any(|s| s.kind == media::StreamKind::Audio)
        {
            return Err(Error::InvalidResponse);
        }
        let target = self.0.cache_path(source);
        let publish = || -> Music<()> {
            self.admit_target(&target)?;
            fs::rename(temp, &target)?;
            Ok(())
        };
        if let Some(ctx) = fence {
            // Cancellation is decided at the job fence; once sealed the rename finishes.
            let mut renamed = None;
            ctx.commit_publication::<_, engine::RuntimeError>(|| {
                renamed = Some(publish());
                Ok(())
            })
            .map_err(|_| Error::Cancelled)?;
            renamed.ok_or(Error::Cancelled)??;
        } else {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            publish()?;
        }
        Ok(CachedMusic {
            path: target,
            ..cached
        })
    }
}
