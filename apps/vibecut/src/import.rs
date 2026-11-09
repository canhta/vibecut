//! Background media import: probe/hash on the engine job runner, publish one
//! typed event per file, then generate bounded thumbnails in a second job. The
//! content-keyed thumbnail cache lives in the engine artifact policy; already
//! imported assets survive cancel, failure or a failed thumbnail.
use crate::{Error, Result};
use engine::cache::{ArtifactKind, CacheKey, KeyInputs};
use engine::jobs::{
    CancelToken, JobId, JobKind, JobRunner, JobSpec, JobStage, JobState, ResourceClass,
    RunnerConfig, SystemClock,
};
use engine::thumbnails::{RetainedThumbnail, ThumbnailCache, thumbnail_key};
use engine::{ByteBudget, ByteLease, ByteOwner};
use media::{AssetId, ImportedSource, MediaError, SourceAsset, THUMBNAIL_MAX_EDGE, Thumbnail};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
use time::Tick;

/// Native picker and drop filters. Probing remains the authority; an extension
/// outside this list can still be dropped and produce a typed media refusal.
pub const MEDIA_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "m4v", "mkv", "webm", "avi", "mpg", "mpeg", "mp3", "wav", "m4a", "aac", "flac",
    "ogg", "opus", "png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff",
];
/// One drop/picker batch is bounded so a hostile or accidental selection cannot
/// create an unbounded path/result working set.
pub const MAX_IMPORT_FILES: usize = 128;
const MAX_IMPORT_PATH_BYTES: usize = 32 * 1024;
/// Bounded producer/consumer handoff. A paused UI blocks the worker instead of
/// accumulating completed metadata without limit.
const EVENT_QUEUE: usize = 8;
/// Conservative work allowance: 256 MiB for bounded single-worker native
/// decoder references/scratch, 64 MiB packets, 32 MiB probe IO and 96 MiB for
/// demux/scaler/hash/handoff overhead. Retained images have separate leases.
const WORK_ALLOWANCE: u64 = 448 << 20;
/// One thumbnail's cache payload is far below this; the decode itself is capped
/// by media's stricter thumbnail/recovery decoder ceiling.
const MAX_THUMBNAIL_ENTRIES: usize = 4096;

/// Injected media boundary so tests drive import outcomes without decoding files.
/// Production passes the same cooperative token into media's interruptible probe
/// and full-content hash.
pub trait ImportProbe: Send + Sync + 'static {
    fn import(
        &self,
        id: AssetId,
        path: PathBuf,
        cancelled: Arc<AtomicBool>,
    ) -> media::Result<ImportedSource>;
    fn import_cached(
        &self,
        id: AssetId,
        path: PathBuf,
        cancelled: Arc<AtomicBool>,
        _cached: &mut dyn FnMut(media::ContentHash) -> Option<media::SourceProbe>,
        _progress: &mut dyn FnMut(u64, u64),
    ) -> media::Result<(ImportedSource, Option<Thumbnail>)> {
        self.import(id, path, cancelled)
            .map(|imported| (imported, None))
    }
}

/// Production probe: the existing media owner canonicalizes, probes, hashes and
/// opens decoders once to report capability.
pub struct MediaProbe;
impl ImportProbe for MediaProbe {
    fn import(
        &self,
        id: AssetId,
        path: PathBuf,
        cancelled: Arc<AtomicBool>,
    ) -> media::Result<ImportedSource> {
        SourceAsset::import_with_summary(id, path, cancelled)
    }
    fn import_cached(
        &self,
        id: AssetId,
        path: PathBuf,
        cancelled: Arc<AtomicBool>,
        cached: &mut dyn FnMut(media::ContentHash) -> Option<media::SourceProbe>,
        progress: &mut dyn FnMut(u64, u64),
    ) -> media::Result<(ImportedSource, Option<Thumbnail>)> {
        SourceAsset::import_prepared(id, path, cancelled, cached, progress)
    }
}

/// Injected thumbnail generator. Production reuses media's one frame decoder.
pub trait ThumbnailSource: Send + Sync + 'static {
    fn thumbnail(
        &self,
        asset: &SourceAsset,
        at: Tick,
        max_edge: u32,
        cancelled: Arc<AtomicBool>,
    ) -> media::Result<Thumbnail>;
}

pub struct MediaThumbnails;
impl ThumbnailSource for MediaThumbnails {
    fn thumbnail(
        &self,
        asset: &SourceAsset,
        at: Tick,
        max_edge: u32,
        cancelled: Arc<AtomicBool>,
    ) -> media::Result<Thumbnail> {
        media::thumbnail(asset, at, max_edge, cancelled)
    }
}

/// One file that did not produce a library asset. `media` retains the typed
/// media error; `reason` is the displayed summary (and the fallback for local
/// engine refusals that have no media type).
#[derive(Clone, Debug)]
pub struct ImportFailure {
    pub path: PathBuf,
    pub reason: String,
    pub media: Option<Arc<MediaError>>,
}
impl ImportFailure {
    pub fn from_media(path: PathBuf, error: MediaError) -> Self {
        Self {
            path,
            reason: error.to_string(),
            media: Some(Arc::new(error)),
        }
    }
    pub fn local(path: PathBuf, reason: String) -> Self {
        Self {
            path,
            reason,
            media: None,
        }
    }
}

#[derive(Debug)]
pub enum ThumbnailOutcome {
    Ready(RetainedThumbnail),
    /// The asset stays usable; the library shows a placeholder.
    Failed(Arc<MediaError>),
}

/// Events are applied by the UI thread, so project mutation/cancellation keep
/// the same single-owner boundary as every other edit.
#[derive(Debug)]
pub enum ImportEvent {
    Probing {
        id: AssetId,
        path: PathBuf,
    },
    Imported(ImportedSource),
    Failed(ImportFailure),
    Thumbnail {
        asset: AssetId,
        outcome: ThumbnailOutcome,
    },
    Done {
        cancelled: bool,
        /// Typed terminal runner failure, e.g. worker unwind. File-level media
        /// refusals arrive as `Failed` and do not close the batch.
        error: Option<Arc<engine::EditError>>,
    },
}

impl ImportEvent {
    fn done(&self) -> Option<(bool, Option<Arc<engine::EditError>>)> {
        match self {
            Self::Done { cancelled, error } => Some((*cancelled, error.clone())),
            _ => None,
        }
    }
}

struct Job {
    id: JobId,
    cancel: CancelToken,
    cancelled: Arc<AtomicBool>,
    receive: mpsc::Receiver<ImportEvent>,
    finished: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Probe,
    Thumbnail,
}

/// Cache hits are answered from the engine cache; the small in-memory map keeps
/// repeated imports in one session from touching the disk again.
#[derive(Default)]
struct ThumbnailStore {
    cache: Option<ThumbnailCache>,
    epoch: u64,
    memo: BTreeMap<CacheKey, RetainedThumbnail>,
}
impl ThumbnailStore {
    fn probe(&self, content: media::ContentHash) -> Option<media::SourceProbe> {
        let key = thumbnail_key(content, Tick::ZERO, THUMBNAIL_MAX_EDGE).ok()?;
        self.memo.get(&key).map(|image| image.probe().clone())
    }
    fn retain(
        store: &Mutex<Self>,
        key: CacheKey,
        asset: &SourceAsset,
        thumbnail: Thumbnail,
        budget: &ByteBudget,
    ) -> Option<RetainedThumbnail> {
        // Budget admission can evict disk artifacts, so never hold the memo lock
        // during admission or cache IO. UI undo only needs this short memo lock.
        loop {
            match RetainedThumbnail::new(thumbnail.clone(), asset, budget) {
                Ok(image) => {
                    let mut memo = store.lock().ok()?;
                    while memo.memo.len() >= 64 {
                        memo.memo.pop_first();
                    }
                    memo.memo.insert(key, image.clone());
                    return Some(image);
                }
                Err(_) => {
                    store.lock().ok()?.memo.pop_first()?;
                }
            }
        }
    }
}
/// A job temporarily owns the disk cache. Its guard restores ownership even on
/// cancellation/unwind, without holding a UI-shared lock across filesystem IO.
struct ThumbnailDisk {
    store: Arc<Mutex<ThumbnailStore>>,
    cache: Option<ThumbnailCache>,
    epoch: u64,
}
impl ThumbnailDisk {
    fn take(store: Arc<Mutex<ThumbnailStore>>) -> engine::Result<Self> {
        let (cache, epoch) = {
            let mut memo = store.lock().map_err(|_| engine::EditError::JobPoisoned)?;
            (memo.cache.take(), memo.epoch)
        };
        Ok(Self {
            store,
            cache,
            epoch,
        })
    }
    fn probe(&mut self, content: media::ContentHash) -> Option<media::SourceProbe> {
        let memory = self.store.lock().ok()?.probe(content);
        memory.or_else(|| {
            self.cache
                .as_mut()
                .and_then(|cache| cache.get_probe(content))
        })
    }
    fn lookup(
        &mut self,
        asset: &SourceAsset,
        at: Tick,
    ) -> media::Result<Option<RetainedThumbnail>> {
        let key = thumbnail_key(asset.content_hash(), at, THUMBNAIL_MAX_EDGE)
            .map_err(|_| MediaError::Invalid("thumbnail key"))?;
        if let Some(image) = self
            .store
            .lock()
            .map_err(|_| MediaError::Invalid("thumbnail cache lock"))?
            .memo
            .get(&key)
            .cloned()
        {
            return Ok(Some(image));
        }
        let Some(cache) = &mut self.cache else {
            return Ok(None);
        };
        match cache.get(asset, at, THUMBNAIL_MAX_EDGE) {
            Ok(Some(image)) => {
                let mut memo = self
                    .store
                    .lock()
                    .map_err(|_| MediaError::Invalid("thumbnail cache lock"))?;
                while memo.memo.len() >= 64 {
                    memo.memo.pop_first();
                }
                memo.memo.insert(key, image.clone());
                Ok(Some(image))
            }
            Ok(None) => Ok(None),
            Err(engine::EditError::BudgetFull | engine::EditError::ArtifactTooLarge) => {
                Err(MediaError::TooLarge("retained thumbnail budget"))
            }
            Err(error) => {
                tracing::warn!(category = "thumbnail_cache_read", error = %error,"thumbnail cache miss");
                Ok(None)
            }
        }
    }
    fn insert(&mut self, asset: &SourceAsset, at: Tick, thumbnail: &Thumbnail) {
        if let Some(cache) = &mut self.cache
            && let Err(error) = cache.put(asset, at, THUMBNAIL_MAX_EDGE, thumbnail)
        {
            tracing::warn!(category = "thumbnail_cache_write", error = %error,"thumbnail not cached");
        }
    }
}
impl Drop for ThumbnailDisk {
    fn drop(&mut self) {
        let old = if let Ok(mut memo) = self.store.lock() {
            if memo.epoch == self.epoch {
                std::mem::replace(&mut memo.cache, self.cache.take())
            } else {
                None
            }
        } else {
            None
        };
        // Close any replaced/root-invalidated cache outside the memo lock.
        drop(old);
    }
}

/// One bounded import batch. The runner owns worker lifetime and cancellation;
/// this queue only exposes typed events until the batch closes.
pub struct ImportQueue {
    /// Declared before the runner so dropping the receiver unblocks a worker
    /// waiting on the bounded event channel before `JobRunner` joins it.
    job: Option<Job>,
    runner: JobRunner,
    probe: Arc<dyn ImportProbe>,
    thumbnailer: Arc<dyn ThumbnailSource>,
    thumbnails: Arc<Mutex<ThumbnailStore>>,
    thumbnail_root: Option<PathBuf>,
    budget: ByteBudget,
    work: Option<ByteLease>,
    phase: Phase,
    pending: Vec<SourceAsset>,
    total: usize,
}
impl Drop for ImportQueue {
    fn drop(&mut self) {
        if let Some(job) = self.job.take() {
            job.cancelled.store(true, Ordering::Release);
            job.cancel.cancel();
            let Job { receive, .. } = job;
            // Release a blocked sender, but keep its allowance until it exits.
            drop(receive);
            self.runner.shutdown();
        }
        self.work = None;
    }
}
impl ImportQueue {
    pub fn new(budget: ByteBudget) -> Result<Self> {
        let runner = JobRunner::new(
            RunnerConfig {
                concurrency: [1, 1, 1],
                queue: 2,
                retained: 2,
                network_attempts: 1,
                backoff: Duration::from_secs(1),
                progress_interval: Duration::from_millis(20),
            },
            budget.clone(),
            Arc::new(SystemClock::default()),
        )?;
        Ok(Self {
            job: None,
            runner,
            probe: Arc::new(MediaProbe),
            thumbnailer: Arc::new(MediaThumbnails),
            thumbnails: Arc::new(Mutex::new(ThumbnailStore::default())),
            thumbnail_root: None,
            budget,
            work: None,
            phase: Phase::Idle,
            pending: Vec::new(),
            total: 0,
        })
    }
    pub fn set_probe(&mut self, probe: Arc<dyn ImportProbe>) {
        self.probe = probe;
    }
    pub fn set_thumbnailer(&mut self, thumbnailer: Arc<dyn ThumbnailSource>) {
        self.thumbnailer = thumbnailer;
    }
    /// Persistent cache directory; without one, thumbnails are memoized for the
    /// session only. Opening is deferred and a busy/corrupt directory degrades to
    /// memo-only rather than failing the import.
    pub fn set_thumbnail_root(&mut self, root: PathBuf) {
        if self.thumbnail_root.as_ref() != Some(&root) {
            self.thumbnail_root = Some(root);
            let old = if let Ok(mut store) = self.thumbnails.lock() {
                store.epoch = store.epoch.saturating_add(1);
                store.cache.take()
            } else {
                None
            };
            drop(old);
        }
    }
    pub fn evict_thumbnails(&mut self) {
        if let Ok(mut store) = self.thumbnails.lock() {
            store.memo.clear();
        }
    }
    /// Drop memo pins when library ownership ends (including undo).
    pub fn retain_assets(&mut self, assets: &[SourceAsset]) {
        let keys: BTreeSet<_> = assets
            .iter()
            .filter_map(|a| thumbnail_key(a.content_hash(), Tick::ZERO, THUMBNAIL_MAX_EDGE).ok())
            .collect();
        if let Ok(mut store) = self.thumbnails.lock() {
            store.memo.retain(|key, _| keys.contains(key));
        }
    }
    pub fn active(&self) -> bool {
        self.job.is_some()
    }
    pub fn total(&self) -> usize {
        self.total
    }
    /// Probe fraction reported by the job runner while files are hashed and
    /// probed, or `None` outside the probe phase.
    pub fn progress(&self) -> Option<f32> {
        if self.phase != Phase::Probe {
            return None;
        }
        let job = self.job.as_ref()?;
        self.runner
            .snapshot(job.id)
            .map(|snapshot| snapshot.progress)
    }
    pub fn cancel(&self) {
        if let Some(job) = &self.job {
            job.cancelled.store(true, Ordering::Release);
            job.cancel.cancel();
        }
    }
    /// Probe files in order. Successes are published immediately, so a later
    /// refusal or cancel cannot roll back earlier assets. The shared budget must
    /// admit the batch's probe/work ceiling before any path is opened.
    pub fn start(&mut self, paths: Vec<PathBuf>, first_id: u64) -> Result<()> {
        if self.job.is_some() {
            return Err(Error::Invalid("import already in progress"));
        }
        if paths.len() > MAX_IMPORT_FILES {
            return Err(Error::Invalid("import batch limit"));
        }
        if paths
            .iter()
            .any(|path| path.as_os_str().len() > MAX_IMPORT_PATH_BYTES)
        {
            return Err(Error::Invalid("import path limit"));
        }
        let mut seen = BTreeSet::new();
        let paths: Vec<PathBuf> = paths
            .into_iter()
            .filter(|path| seen.insert(path.clone()))
            .collect();
        if paths.is_empty() {
            return Ok(());
        }
        self.open_thumbnail_cache();
        let work = self.budget.reserve(ByteOwner::JobResults, WORK_ALLOWANCE)?;
        let total = paths.len();
        self.total = total;
        self.pending.clear();
        self.phase = Phase::Probe;
        let (send, receive) = mpsc::sync_channel(EVENT_QUEUE);
        let probe = self.probe.clone();
        let store = self.thumbnails.clone();
        let budget = self.budget.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancelled.clone();
        let key = job_key(ArtifactKind::Preview, "media-probe")?;
        let submitted = self.runner.submit(
            JobSpec {
                kind: JobKind::Probe,
                class: ResourceClass::Cpu,
                input: key,
                bytes: Default::default(),
            },
            move |ctx| {
                let mut disk = ThumbnailDisk::take(store.clone())?;
                let mut next = first_id;
                for (index, path) in paths.iter().enumerate() {
                    if ctx.cancelled() {
                        worker_cancel.store(true, Ordering::Release);
                    }
                    if worker_cancel.load(Ordering::Acquire) {
                        return finish(&send, true, None);
                    }
                    let report = |fraction: f32| {
                        ctx.progress(
                            JobStage::Probe,
                            (index as f32 + fraction) / total.max(1) as f32,
                        )
                        .ok();
                    };
                    report(0.0);
                    if send
                        .send(ImportEvent::Probing {
                            id: AssetId(next),
                            path: path.clone(),
                        })
                        .is_err()
                    {
                        return Ok(Vec::new());
                    }
                    let mut cached = |hash| disk.probe(hash);
                    let mut progress = |hashed: u64, length: u64| {
                        report(probe_fraction(hashed, length));
                    };
                    match probe.import_cached(
                        AssetId(next),
                        path.clone(),
                        worker_cancel.clone(),
                        &mut cached,
                        &mut progress,
                    ) {
                        Ok((imported, recovered)) => {
                            if let Some(image) = recovered {
                                disk.insert(&imported.asset, Tick::ZERO, &image);
                                if let Ok(key) = thumbnail_key(
                                    imported.asset.content_hash(),
                                    Tick::ZERO,
                                    THUMBNAIL_MAX_EDGE,
                                ) {
                                    ThumbnailStore::retain(
                                        &store,
                                        key,
                                        &imported.asset,
                                        image,
                                        &budget,
                                    );
                                }
                            }
                            next = next.checked_add(1).ok_or(engine::EditError::Limit)?;
                            if send.send(ImportEvent::Imported(imported)).is_err() {
                                return Ok(Vec::new());
                            }
                        }
                        Err(MediaError::Cancelled)
                            if worker_cancel.load(Ordering::Acquire) || ctx.cancelled() =>
                        {
                            return finish(&send, true, None);
                        }
                        Err(error) => {
                            if send
                                .send(ImportEvent::Failed(ImportFailure::from_media(
                                    path.clone(),
                                    error,
                                )))
                                .is_err()
                            {
                                return Ok(Vec::new());
                            }
                        }
                    }
                }
                finish(&send, ctx.cancelled(), None)
            },
        );
        let (id, cancel) = submitted?;
        self.work = Some(work);
        self.job = Some(Job {
            id,
            cancel,
            cancelled,
            receive,
            finished: false,
        });
        Ok(())
    }
    fn open_thumbnail_cache(&mut self) {
        let Some(root) = self.thumbnail_root.clone() else {
            return;
        };
        let epoch = match self.thumbnails.lock() {
            Ok(store) if store.cache.is_none() => store.epoch,
            _ => return,
        };
        match ThumbnailCache::open(&root, self.budget.clone(), MAX_THUMBNAIL_ENTRIES) {
            Ok(cache) => {
                let mut cache = Some(cache);
                if let Ok(mut store) = self.thumbnails.lock()
                    && store.epoch == epoch
                    && store.cache.is_none()
                {
                    store.cache = cache.take();
                }
            }
            Err(error) => {
                tracing::warn!(category = "thumbnail_cache_open", error = %error,"thumbnail cache unavailable")
            }
        }
    }
    /// Drain completed events. A worker panic or terminal runner state still
    /// closes the batch with one typed `Done`, so the UI cannot stay busy.
    pub fn poll(&mut self) -> std::result::Result<Vec<ImportEvent>, Arc<engine::EditError>> {
        if let Err(error) = self.runner.tick() {
            if let Some(job) = self.job.take() {
                let _ = self.runner.take_output(job.id);
            }
            self.finish();
            return Err(Arc::new(error));
        }
        let mut events = Vec::new();
        let mut terminal = None;
        if let Some(job) = &mut self.job {
            while let Ok(event) = job.receive.try_recv() {
                if let Some(done) = event.done() {
                    job.finished = true;
                    terminal = Some(done);
                } else {
                    if let ImportEvent::Imported(imported) = &event
                        && needs_thumbnail(imported)
                    {
                        self.pending.push(imported.asset.clone());
                    }
                    events.push(event);
                }
            }
            if !job.finished
                && let Some(snapshot) = self.runner.snapshot(job.id)
                && matches!(
                    snapshot.state,
                    JobState::Completed | JobState::Failed | JobState::Cancelled
                )
            {
                job.finished = true;
                terminal = Some((
                    snapshot.state == JobState::Cancelled,
                    if snapshot.state == JobState::Failed {
                        snapshot.error
                    } else {
                        None
                    },
                ));
            }
        }
        let Some((cancelled, error)) = terminal else {
            return Ok(events);
        };
        if let Some(job) = self.job.take() {
            let _ = self.runner.take_output(job.id);
        }
        let phase = self.phase;
        self.phase = Phase::Idle;
        if phase == Phase::Probe && !cancelled && error.is_none() && !self.pending.is_empty() {
            match self.start_thumbnails() {
                Ok(()) => {}
                Err(error) => {
                    events.push(ImportEvent::Done {
                        cancelled: false,
                        error: Some(Arc::new(error)),
                    });
                    self.finish();
                }
            }
            return Ok(events);
        }
        events.push(ImportEvent::Done { cancelled, error });
        self.finish();
        Ok(events)
    }
    fn finish(&mut self) {
        self.work = None;
        self.pending.clear();
        self.phase = Phase::Idle;
        self.total = 0;
    }
    /// Thumbnails run as a second registered job after probing, so the library
    /// can show `probing` badges before decode starts.
    fn start_thumbnails(&mut self) -> std::result::Result<(), engine::EditError> {
        let assets = std::mem::take(&mut self.pending);
        let total = assets.len();
        self.phase = Phase::Thumbnail;
        let (send, receive) = mpsc::sync_channel(EVENT_QUEUE);
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancelled.clone();
        let store = self.thumbnails.clone();
        let thumbnailer = self.thumbnailer.clone();
        let budget = self.budget.clone();
        let key = job_key(ArtifactKind::Thumbnail, "media-thumbnail")?;
        let submitted = self.runner.submit(
            JobSpec {
                kind: JobKind::Thumbnail,
                class: ResourceClass::Cpu,
                input: key,
                bytes: Default::default(),
            },
            move |ctx| {
                let mut disk = ThumbnailDisk::take(store.clone())?;
                for (index, asset) in assets.iter().enumerate() {
                    if ctx.cancelled() {
                        worker_cancel.store(true, Ordering::Release);
                    }
                    if worker_cancel.load(Ordering::Acquire) {
                        return finish(&send, true, None);
                    }
                    ctx.progress(JobStage::Thumbnail, index as f32 / total.max(1) as f32)
                        .ok();
                    let outcome = match disk.lookup(asset, Tick::ZERO) {
                        Ok(Some(image)) => ThumbnailOutcome::Ready(image),
                        Err(error) => ThumbnailOutcome::Failed(Arc::new(error)),
                        Ok(None) => match thumbnailer.thumbnail(
                            asset,
                            Tick::ZERO,
                            THUMBNAIL_MAX_EDGE,
                            worker_cancel.clone(),
                        ) {
                            Ok(image) => {
                                disk.insert(asset, Tick::ZERO, &image);
                                let key = thumbnail_key(
                                    asset.content_hash(),
                                    Tick::ZERO,
                                    THUMBNAIL_MAX_EDGE,
                                )?;
                                match ThumbnailStore::retain(&store, key, asset, image, &budget) {
                                    Some(image) => ThumbnailOutcome::Ready(image),
                                    None => ThumbnailOutcome::Failed(Arc::new(
                                        MediaError::TooLarge("retained thumbnail budget"),
                                    )),
                                }
                            }
                            Err(MediaError::Cancelled) => return finish(&send, true, None),
                            Err(error) => ThumbnailOutcome::Failed(Arc::new(error)),
                        },
                    };
                    if send
                        .send(ImportEvent::Thumbnail {
                            asset: asset.id(),
                            outcome,
                        })
                        .is_err()
                    {
                        return Ok(Vec::new());
                    }
                }
                finish(&send, ctx.cancelled(), None)
            },
        );
        match submitted {
            Ok((id, cancel)) => {
                self.job = Some(Job {
                    id,
                    cancel,
                    cancelled,
                    receive,
                    finished: false,
                });
                Ok(())
            }
            Err(error) => {
                self.phase = Phase::Idle;
                Err(error)
            }
        }
    }
}

/// Video and image assets need a thumbnail; audio-only assets are ready at once.
pub fn needs_thumbnail(imported: &ImportedSource) -> bool {
    imported
        .metadata()
        .is_ok_and(|metadata| metadata.needs_thumbnail())
        && imported.capabilities.is_decodable()
}

fn job_key(
    kind: ArtifactKind,
    model: &'static str,
) -> std::result::Result<CacheKey, engine::EditError> {
    CacheKey::new(KeyInputs {
        kind,
        source: [0; 32],
        provider: "desktop",
        model,
        version: "1",
        settings: &[],
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
}

fn finish(
    send: &mpsc::SyncSender<ImportEvent>,
    cancelled: bool,
    error: Option<Arc<engine::EditError>>,
) -> std::result::Result<Vec<u8>, engine::EditError> {
    if send.send(ImportEvent::Done { cancelled, error }).is_err() {
        return Err(engine::EditError::Cancelled);
    }
    Ok(Vec::new())
}

/// One file's hash fraction, held below the whole file: probing and decoder
/// checks still follow the hash, so the overlay reaches 100% only when the
/// batch finishes. A file that grew mid-read stays at the cap, so the batch
/// never moves back.
fn probe_fraction(hashed: u64, length: u64) -> f32 {
    const HASHED_FILE: f64 = 0.99;
    if length == 0 {
        return HASHED_FILE as f32;
    }
    ((hashed as f64 / length as f64).clamp(0.0, 1.0) * HASHED_FILE) as f32
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod retention_tests {
    use super::*;
    fn fixture(hash: u8) -> SourceAsset {
        SourceAsset::try_from(media::AssetData {
            ephemeral: false,
            id: AssetId(hash as u64),
            path: std::env::temp_dir().join("retained-thumbnail"),
            content_hash: media::ContentHash([hash; 32]),
            probe: media::SourceProbe {
                format: "fixture".into(),
                duration: None,
                streams: Vec::new(),
            },
        })
        .unwrap()
    }
    fn image() -> Thumbnail {
        Thumbnail {
            width: 2,
            height: 1,
            frame: Tick::ZERO,
            pixels: vec![0; 8],
        }
    }
    #[test]
    fn thumbnail_retention_evicts_under_pressure_and_bounds_entries() {
        let calibration = ByteBudget::new(1 << 20).unwrap();
        let retained = RetainedThumbnail::new(image(), &fixture(0), &calibration).unwrap();
        let charge = calibration.snapshot().unwrap().used;
        drop(retained);
        for (bytes, expected) in [(charge * 2, 2), (1 << 20, 64)] {
            let budget = ByteBudget::new(bytes).unwrap();
            let store = Mutex::new(ThumbnailStore::default());
            for hash in 0..100 {
                let asset = fixture(hash);
                let key = thumbnail_key(asset.content_hash(), Tick::ZERO, 320).unwrap();
                assert!(ThumbnailStore::retain(&store, key, &asset, image(), &budget).is_some());
                assert!(budget.snapshot().unwrap().used <= bytes);
            }
            assert_eq!(store.lock().unwrap().memo.len(), expected);
            assert_eq!(budget.snapshot().unwrap().used, expected as u64 * charge);
            assert_eq!(
                store.lock().unwrap().probe(fixture(99).content_hash()),
                Some(fixture(99).probe().clone()),
                "memory hits retain geometry without any disk cache"
            );
            drop(store);
            assert_eq!(budget.snapshot().unwrap().used, 0);
        }
    }
}
