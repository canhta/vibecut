//! Resolves preview video reads to cached proxies while export keeps the source.
use media::{AssetData, ContentHash, SourceAsset, SourceProbe};
use project::ItemId;
use render::{AudioBlock, ExecutionHooks, FileSources, Image, SourceProvider, UnavailableReason};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};
use time::Tick;

/// Why preview read a source although proxies are on and one was expected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyFallback {
    /// The cached proxy file vanished or changed size, for example after the
    /// cache was cleared. The source plays and the proxy can be generated again.
    Missing,
}

struct Entry {
    asset: SourceAsset,
    bytes: u64,
}

#[derive(Default)]
struct State {
    enabled: AtomicBool,
    entries: RwLock<BTreeMap<[u8; 32], Entry>>,
    fallbacks: Mutex<BTreeMap<[u8; 32], ProxyFallback>>,
}

/// The proxies preview may read, keyed by source content hash, and the project's
/// on/off choice. Cheap to clone; every clone shares one state, so a toggle takes
/// effect on the next frame request without reopening the project. It records
/// no path anywhere a project can see: the project keeps naming its sources.
#[derive(Clone, Default)]
pub struct ProxyIndex(Arc<State>);

impl ProxyIndex {
    pub fn set_enabled(&self, enabled: bool) {
        self.0.enabled.store(enabled, Ordering::Release);
    }
    pub fn enabled(&self) -> bool {
        self.0.enabled.load(Ordering::Acquire)
    }
    /// Offer the proxy file of `source`. Its probe is read once here, never per
    /// frame. A later offer for the same content replaces the earlier one.
    pub fn adopt(&self, source: &SourceAsset, path: PathBuf, bytes: u64) -> media::Result<()> {
        let probe = SourceProbe::from(&media::probe(&path)?);
        let asset = SourceAsset::try_from(AssetData {
            ephemeral: true,
            id: source.id(),
            path,
            content_hash: source.content_hash(),
            probe,
        })?;
        let key = source.content_hash().0;
        if let Ok(mut entries) = self.0.entries.write() {
            entries.insert(key, Entry { asset, bytes });
        }
        if let Ok(mut fallbacks) = self.0.fallbacks.lock() {
            fallbacks.remove(&key);
        }
        Ok(())
    }
    /// Stop offering the proxy of this content, for example after a clear.
    pub fn forget(&self, content: ContentHash) {
        if let Ok(mut entries) = self.0.entries.write() {
            entries.remove(&content.0);
        }
    }
    pub fn contains(&self, content: ContentHash) -> bool {
        self.0
            .entries
            .read()
            .is_ok_and(|entries| entries.contains_key(&content.0))
    }
    /// Sources preview fell back from, with the typed reason, until the next
    /// offer for the same content.
    pub fn fallbacks(&self) -> Vec<(ContentHash, ProxyFallback)> {
        self.0.fallbacks.lock().map_or_else(
            |_| Vec::new(),
            |fallbacks| {
                fallbacks
                    .iter()
                    .map(|(key, reason)| (ContentHash(*key), *reason))
                    .collect()
            },
        )
    }
    /// The proxy to read instead of `source`: `None` when proxies are off, none
    /// was offered, or the offered file is gone, in which case the entry is
    /// dropped and the reason recorded.
    fn resolve(&self, source: &SourceAsset) -> Option<SourceAsset> {
        if !self.enabled() {
            return None;
        }
        let key = source.content_hash().0;
        let (asset, bytes) = {
            let entries = self.0.entries.read().ok()?;
            let entry = entries.get(&key)?;
            (entry.asset.clone(), entry.bytes)
        };
        if std::fs::metadata(asset.path()).is_ok_and(|m| m.is_file() && m.len() == bytes) {
            return Some(asset);
        }
        if let Ok(mut entries) = self.0.entries.write() {
            entries.remove(&key);
        }
        if let Ok(mut fallbacks) = self.0.fallbacks.lock() {
            fallbacks.insert(key, ProxyFallback::Missing);
        }
        None
    }
}

/// The preview source provider: video comes from the proxy when `ProxyIndex`
/// offers one for the asset's content, everything else, including every audio
/// read and the availability check, from the source. Export never constructs it.
pub struct ProxySources {
    inner: FileSources,
    index: ProxyIndex,
}

impl ProxySources {
    pub fn new(index: ProxyIndex) -> Self {
        Self {
            inner: FileSources::default(),
            index,
        }
    }
}

impl SourceProvider for ProxySources {
    fn status(
        &mut self,
        asset: &SourceAsset,
        hash: bool,
        hooks: &dyn ExecutionHooks,
    ) -> render::Result<Option<UnavailableReason>> {
        self.inner.status(asset, hash, hooks)
    }
    fn begin_video(&mut self, active: &[ItemId]) {
        self.inner.begin_video(active);
    }
    fn begin_audio(&mut self, active: &[ItemId]) {
        self.inner.begin_audio(active);
    }
    fn finish_audio(&mut self, item: ItemId) {
        self.inner.finish_audio(item);
    }
    fn video(
        &mut self,
        asset: &SourceAsset,
        item: ItemId,
        at: Tick,
        hooks: &dyn ExecutionHooks,
    ) -> render::Result<Option<Image>> {
        match self.index.resolve(asset) {
            Some(proxy) => self.inner.video(&proxy, item, at, hooks),
            None => self.inner.video(asset, item, at, hooks),
        }
    }
    fn video_window(
        &mut self,
        asset: &SourceAsset,
        item: ItemId,
        at: Tick,
        window: Option<std::ops::Range<u32>>,
        hooks: &dyn ExecutionHooks,
    ) -> render::Result<Option<Image>> {
        match self.index.resolve(asset) {
            // The window is in source columns; a proxy raster is smaller, so
            // the whole frame is read and the compositor scales it.
            Some(proxy) => self.inner.video(&proxy, item, at, hooks),
            None => self.inner.video_window(asset, item, at, window, hooks),
        }
    }
    fn audio(
        &mut self,
        asset: &SourceAsset,
        item: ItemId,
        start: i64,
        frames: usize,
        rate: u32,
        hooks: &dyn ExecutionHooks,
    ) -> render::Result<AudioBlock> {
        self.inner.audio(asset, item, start, frames, rate, hooks)
    }
}
