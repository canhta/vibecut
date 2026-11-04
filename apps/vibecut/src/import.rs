//! Background media import: probe/hash on the engine job runner, publish one
//! typed event per file, and keep already-imported assets on cancel/failure.
use crate::{Error, Result};
use engine::cache::{ArtifactKind, CacheKey, KeyInputs};
use engine::jobs::{
    CancelToken, JobId, JobKind, JobRunner, JobSpec, JobState, ResourceClass, RunnerConfig,
    SystemClock,
};
use engine::{ByteBudget, ByteLease, ByteOwner};
use media::{AssetId, MediaError, SourceAsset};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

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
/// Conservative probe/work ceiling, matching render's 256 MiB source-probe
/// admission. The lease covers FFmpeg probe internals, the hash buffer and the
/// bounded queued metadata for the life of the batch.
const WORK_ALLOWANCE: u64 = 256 << 20;

/// Injected media boundary so tests drive import outcomes without decoding files.
/// Production passes the same cooperative token into media's interruptible probe
/// and full-content hash.
pub trait ImportProbe: Send + Sync + 'static {
    fn import(
        &self,
        id: AssetId,
        path: PathBuf,
        cancelled: Arc<AtomicBool>,
    ) -> media::Result<SourceAsset>;
}

/// Production probe: the existing media owner canonicalizes, probes and hashes.
pub struct MediaProbe;
impl ImportProbe for MediaProbe {
    fn import(
        &self,
        id: AssetId,
        path: PathBuf,
        cancelled: Arc<AtomicBool>,
    ) -> media::Result<SourceAsset> {
        SourceAsset::import_with_cancel(id, path, cancelled)
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

/// Events are applied by the UI thread, so project mutation/cancellation keep
/// the same single-owner boundary as every other edit.
#[derive(Debug)]
pub enum ImportEvent {
    Imported(SourceAsset),
    Failed(ImportFailure),
    Done {
        cancelled: bool,
        /// Typed terminal runner failure, e.g. worker unwind. File-level media
        /// refusals arrive as `Failed` and do not close the batch.
        error: Option<Arc<engine::EditError>>,
    },
}

struct Job {
    id: JobId,
    cancel: CancelToken,
    cancelled: Arc<AtomicBool>,
    receive: mpsc::Receiver<ImportEvent>,
    total: usize,
    finished: bool,
    _work: ByteLease,
}

/// One bounded import batch. The runner owns worker lifetime and cancellation;
/// this queue only exposes typed events until the batch closes.
pub struct ImportQueue {
    /// Declared before the runner so dropping the receiver unblocks a worker
    /// waiting on the bounded event channel before `JobRunner` joins it.
    job: Option<Job>,
    runner: JobRunner,
    probe: Arc<dyn ImportProbe>,
    budget: ByteBudget,
}
impl Drop for ImportQueue {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancelled.store(true, Ordering::Release);
            job.cancel.cancel();
        }
    }
}
impl ImportQueue {
    pub fn new(budget: ByteBudget) -> Result<Self> {
        let runner = JobRunner::new(
            RunnerConfig {
                concurrency: [1, 1, 1],
                queue: 1,
                retained: 1,
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
            budget,
        })
    }
    pub fn set_probe(&mut self, probe: Arc<dyn ImportProbe>) {
        self.probe = probe;
    }
    pub fn active(&self) -> bool {
        self.job.is_some()
    }
    pub fn total(&self) -> usize {
        self.job.as_ref().map_or(0, |job| job.total)
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
        let work = self.budget.reserve(ByteOwner::JobResults, WORK_ALLOWANCE)?;
        let total = paths.len();
        let (send, receive) = mpsc::sync_channel(EVENT_QUEUE);
        let probe = self.probe.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancelled.clone();
        // Import publishes no cached artifact; this key only satisfies the
        // engine job contract, matching the app's other scheduling-only jobs.
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::Preview,
            source: [0; 32],
            provider: "desktop",
            model: "media-import",
            version: "1",
            settings: &[],
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        let submitted = self.runner.submit(
            JobSpec {
                kind: JobKind::Import,
                class: ResourceClass::Cpu,
                input: key,
                bytes: Default::default(),
            },
            move |ctx| {
                let mut next = first_id;
                for path in &paths {
                    if ctx.cancelled() {
                        worker_cancel.store(true, Ordering::Release);
                    }
                    if worker_cancel.load(Ordering::Acquire) {
                        return finish(&send, true, None);
                    }
                    match probe.import(AssetId(next), path.clone(), worker_cancel.clone()) {
                        Ok(asset) => {
                            next = next.checked_add(1).ok_or(engine::EditError::Limit)?;
                            if send.send(ImportEvent::Imported(asset)).is_err() {
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
        self.job = Some(Job {
            id,
            cancel,
            cancelled,
            receive,
            total,
            finished: false,
            _work: work,
        });
        Ok(())
    }
    /// Drain completed events. A worker panic or terminal runner state still
    /// closes the batch with one typed `Done`, so the UI cannot stay busy.
    pub fn poll(&mut self) -> std::result::Result<Vec<ImportEvent>, Arc<engine::EditError>> {
        if let Err(error) = self.runner.tick() {
            if let Some(job) = self.job.take() {
                let _ = self.runner.take_output(job.id);
            }
            return Err(Arc::new(error));
        }
        let mut events = Vec::new();
        let Some(job) = &mut self.job else {
            return Ok(events);
        };
        while let Ok(event) = job.receive.try_recv() {
            if matches!(event, ImportEvent::Done { .. }) {
                job.finished = true;
            }
            events.push(event);
        }
        if !job.finished
            && let Some(snapshot) = self.runner.snapshot(job.id)
            && matches!(
                snapshot.state,
                JobState::Completed | JobState::Failed | JobState::Cancelled
            )
        {
            job.finished = true;
            events.push(ImportEvent::Done {
                cancelled: snapshot.state == JobState::Cancelled,
                error: snapshot.error,
            });
        }
        if job.finished
            && let Some(job) = self.job.take()
        {
            let _ = self.runner.take_output(job.id);
        }
        Ok(events)
    }
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
