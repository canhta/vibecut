//! Background media import: probe/hash on the engine job runner, publish one
//! typed event per file, and keep already-imported assets on cancel/failure.
use crate::{Error, Result};
use engine::ByteBudget;
use engine::cache::{ArtifactKind, CacheKey, KeyInputs};
use engine::jobs::{
    CancelToken, JobId, JobKind, JobRunner, JobSpec, JobState, ResourceClass, RunnerConfig,
    SystemClock,
};
use media::{AssetId, SourceAsset};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, mpsc},
    time::Duration,
};

/// Native picker and drop filters. Probing remains the authority; an extension
/// outside this list can still be dropped and produce a typed media refusal.
pub const MEDIA_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "m4v", "mkv", "webm", "avi", "mpg", "mpeg", "mp3", "wav", "m4a", "aac", "flac",
    "ogg", "opus", "png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff",
];

/// Injected media boundary so tests drive import outcomes without decoding files.
pub trait ImportProbe: Send + Sync + 'static {
    fn import(&self, id: AssetId, path: PathBuf) -> media::Result<SourceAsset>;
}

/// Production probe: the existing media owner canonicalizes, probes and hashes.
pub struct MediaProbe;
impl ImportProbe for MediaProbe {
    fn import(&self, id: AssetId, path: PathBuf) -> media::Result<SourceAsset> {
        SourceAsset::import(id, path)
    }
}

/// One file that did not produce a library asset. `media` retains the typed
/// media error; `reason` is the displayed summary (and the fallback for local
/// engine refusals that have no media type).
#[derive(Clone, Debug)]
pub struct ImportFailure {
    pub path: PathBuf,
    pub reason: String,
    pub media: Option<Arc<media::MediaError>>,
}
impl ImportFailure {
    pub fn from_media(path: PathBuf, error: media::MediaError) -> Self {
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
        error: Option<String>,
    },
}

struct Job {
    id: JobId,
    cancel: CancelToken,
    receive: mpsc::Receiver<ImportEvent>,
    total: usize,
    finished: bool,
}

/// One bounded import batch. The runner owns worker lifetime and cancellation;
/// this queue only exposes typed events until the batch closes.
pub struct ImportQueue {
    runner: JobRunner,
    probe: Arc<dyn ImportProbe>,
    job: Option<Job>,
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
            budget,
            Arc::new(SystemClock::default()),
        )?;
        Ok(Self {
            runner,
            probe: Arc::new(MediaProbe),
            job: None,
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
            job.cancel.cancel();
        }
    }
    /// Probe files in order. Successes are published immediately, so a later
    /// refusal or cancel cannot roll back earlier assets.
    pub fn start(&mut self, paths: Vec<PathBuf>, first_id: u64) -> Result<()> {
        if self.job.is_some() {
            return Err(Error::Invalid("import already in progress"));
        }
        let mut seen = BTreeSet::new();
        let paths: Vec<PathBuf> = paths
            .into_iter()
            .filter(|path| seen.insert(path.clone()))
            .collect();
        if paths.is_empty() {
            return Ok(());
        }
        let total = paths.len();
        let (send, receive) = mpsc::channel();
        let probe = self.probe.clone();
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
        let (id, cancel) = self.runner.submit(
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
                        let _ = send.send(ImportEvent::Done {
                            cancelled: true,
                            error: None,
                        });
                        return Ok(Vec::new());
                    }
                    match probe.import(AssetId(next), path.clone()) {
                        Ok(asset) => {
                            next = next.checked_add(1).ok_or(engine::EditError::Limit)?;
                            if send.send(ImportEvent::Imported(asset)).is_err() {
                                return Ok(Vec::new());
                            }
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
                let cancelled = ctx.cancelled();
                let _ = send.send(ImportEvent::Done {
                    cancelled,
                    error: None,
                });
                Ok(Vec::new())
            },
        )?;
        self.job = Some(Job {
            id,
            cancel,
            receive,
            total,
            finished: false,
        });
        Ok(())
    }
    /// Drain completed events. A worker panic or terminal runner state still
    /// closes the batch with one typed `Done`, so the UI cannot stay busy.
    pub fn poll(&mut self) -> Result<Vec<ImportEvent>> {
        if let Err(error) = self.runner.tick() {
            if let Some(job) = self.job.take() {
                let _ = self.runner.take_output(job.id);
            }
            return Err(error.into());
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
                error: snapshot.error.map(|error| error.to_string()),
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
