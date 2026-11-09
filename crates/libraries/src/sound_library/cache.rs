//! Reuses verified sound preview bytes.
use super::{
    AssetData, CachedSound, ContentHash, Error, MAX_AUDIO_BYTES, NEXT, Ordering, Path, PathBuf,
    Result, Sound, SoundLibrary, SourceAsset, SourceProbe, Tick, Write, fs, url,
};
impl SoundLibrary {
    pub fn cache_path(&self, url: &str) -> PathBuf {
        self.audio.join(format!("{}.audio", cache_key(url)))
    }
}

impl SoundLibrary {
    pub fn cached(&self, url: &str) -> Sound<Option<CachedSound>> {
        let path = self.cache_path(url);
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if !meta.file_type().is_file() || meta.file_type().is_symlink() || meta.len() == 0 {
            return Err(Error::LibraryCorrupt);
        }
        if meta.len() > MAX_AUDIO_BYTES {
            return Err(Error::Limit);
        }
        cached_from_path(path, Some(url)).map(Some)
    }
}

impl SoundLibrary {
    /// Store already downloaded preview bytes. Only complete, probed audio is
    /// published; a failed probe leaves no cache entry.
    pub fn store_cached(&self, source: &str, bytes: &[u8]) -> Sound<CachedSound> {
        if !url(source) || bytes.is_empty() || bytes.len() as u64 > MAX_AUDIO_BYTES {
            return Err(Error::Invalid("cached audio"));
        }
        let path = self.cache_path(source);
        let temp = self.audio.join(format!(
            ".part-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            let cached = cached_from_path(temp.clone(), Some(source))?;
            if cached.probe.duration.is_none_or(|d| d <= Tick::ZERO)
                || !cached
                    .probe
                    .streams
                    .iter()
                    .any(|s| s.kind == media::StreamKind::Audio)
            {
                return Err(Error::InvalidResponse);
            }
            fs::rename(&temp, &path)?;
            cached_from_path(path, Some(source))
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
}

impl SoundLibrary {
    pub(super) fn remove_cached(&self, url: &str) -> Sound<()> {
        match fs::remove_file(self.cache_path(url)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}
fn cache_key(url: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"vibecut-sound-v1");
    hasher.update(url.as_bytes());
    hasher.finalize().to_hex().to_string()
}
impl CachedSound {
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
    pub fn bytes(&self) -> Sound<Vec<u8>> {
        let meta = fs::metadata(&self.path)?;
        if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_AUDIO_BYTES {
            return Err(Error::Limit);
        }
        Ok(fs::read(&self.path)?)
    }
}
pub(super) fn cached_from_path(path: PathBuf, source: Option<&str>) -> Sound<CachedSound> {
    if let Some(source) = source
        && !url(source)
    {
        return Err(Error::Invalid("cached audio URL"));
    }
    let hash = ContentHash::read_with_cancel(&path, || false, |_, _| {})?;
    let probe = media::probe(&path)?;
    Ok(CachedSound {
        path,
        content_hash: hash,
        probe: SourceProbe::from(&probe),
    })
}
impl SoundLibrary {
    pub(super) fn publish_temp(
        &self,
        source: &str,
        temp: &Path,
        cancelled: &dyn Fn() -> bool,
        fence: Option<&engine::jobs::JobContext>,
    ) -> Sound<CachedSound> {
        if cancelled() {
            return Err(Error::Cancelled);
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
        let target = self.cache_path(source);
        if let Some(ctx) = fence {
            // Cancellation is decided at the job fence; once sealed the rename finishes.
            let mut renamed = None;
            ctx.commit_publication(|| {
                renamed = Some(fs::rename(temp, &target));
                Ok(())
            })
            .map_err(|_| Error::Cancelled)?;
            renamed.ok_or(Error::Cancelled)??;
        } else {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            fs::rename(temp, &target)?;
        }
        cached_from_path(self.cache_path(source), Some(source))
    }
}
