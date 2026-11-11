//! Consumes the engine's cache-only clip display queries.
//!
//! The panel asks for ready tiles, bars or a typed state; this owner opens the
//! process media caches, calls `waveform::display` / `thumbnail_strip::display`
//! and submits the registered generation commands when a display is pending.
//! It never decodes media and never builds a cache key.

use crate::editor::timeline::{AssetId, Tick, egui, theme};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

use crate::editor::context::visuals::{
    StripState, StripTile, VisualKey, VisualQuery, VisualRequest, WaveBar, WaveState,
};
use crate::editor::context::{CacheAction, CachePhase, CacheUi};
use engine::cache::{CacheMaintenanceReport, CacheOperation};
use std::sync::atomic::{AtomicBool, Ordering};

struct CacheWorker {
    cancel: Arc<AtomicBool>,
    handle: std::thread::JoinHandle<CacheMaintenanceReport>,
}
impl CacheWorker {
    fn spawn(
        run: impl FnOnce(Arc<AtomicBool>) -> CacheMaintenanceReport + Send + 'static,
    ) -> std::io::Result<Self> {
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let handle = std::thread::Builder::new()
            .name("timeline-cache".into())
            .spawn(move || {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(flag))).unwrap_or_else(
                    |_| CacheMaintenanceReport {
                        cause: Some(engine::RuntimeError::WorkerPanicked.into()),
                        failed: 1,
                        ..Default::default()
                    },
                )
            })?;
        Ok(Self { cancel, handle })
    }
}
/// Reserved texture slots for strip frames; the oldest frame is dropped first.
const MAX_TEXTURES: usize = 512;
/// Loopback policy for the editor's own registered commands.
struct EditorPolicy;
impl engine::control::Policy for EditorPolicy {
    fn authorize(&self, _: &engine::Command, _: &project::Project) -> engine::Result<()> {
        Ok(())
    }
}

/// Texture cache key: asset, cached source position and content identity.
type StripTextures = BTreeMap<(AssetId, Tick, [u8; 32]), egui::TextureHandle>;

/// Production display queries plus the jobs they request.
pub(in crate::editor) struct Media {
    budget: engine::ByteBudget,
    /// The one process cache instance the workspace lends to this editor and
    /// the MCP inbox; `None` until the composition root attaches it.
    access: Option<Arc<Mutex<engine::control::MediaAccess>>>,
    runner: Option<engine::jobs::JobRunner>,
    /// In-flight generation job -> the display request that submitted it.
    jobs: BTreeMap<engine::jobs::JobId, VisualRequest>,
    /// One request per cache family until its job settles.
    pending: RefCell<BTreeMap<VisualKey, VisualRequest>>,
    /// Terminal generation failures stay typed instead of retrying every frame.
    failed: RefCell<BTreeSet<VisualKey>>,
    queued: RefCell<Vec<VisualRequest>>,
    textures: RefCell<StripTextures>,
    request_id: u64,
    /// False reserves bounded recovery metadata; true means a job was admitted.
    recovery: RefCell<BTreeMap<VisualKey, bool>>,
    visual_cause: RefCell<Option<Arc<engine::Error>>>,
    cache: CacheUi,
    cache_worker: Option<CacheWorker>,
    cache_operation: CacheOperation,
}

impl Media {
    pub(in crate::editor) fn new(budget: engine::ByteBudget) -> Self {
        Self {
            budget,
            access: None,
            runner: None,
            jobs: BTreeMap::new(),
            pending: RefCell::new(BTreeMap::new()),
            failed: RefCell::new(BTreeSet::new()),
            queued: RefCell::new(Vec::new()),
            textures: RefCell::new(BTreeMap::new()),
            request_id: 0,
            recovery: RefCell::new(BTreeMap::new()),
            visual_cause: RefCell::new(None),
            cache: CacheUi::default(),
            cache_worker: None,
            cache_operation: CacheOperation::Inspect,
        }
    }

    /// Adopt the workspace's one media cache instance.
    pub(in crate::editor) fn set_access(
        &mut self,
        access: Arc<Mutex<engine::control::MediaAccess>>,
    ) {
        self.access = Some(access);
        self.cache.phase = CachePhase::Idle;
    }

    pub(in crate::editor) fn cache_ui(&self) -> CacheUi {
        let mut ui = self.cache.clone();
        if ui.cause.is_none() {
            ui.cause = self.visual_cause.borrow().clone();
        }
        ui
    }

    pub(in crate::editor) fn request_cache(&mut self, action: CacheAction) {
        if action == CacheAction::Cancel {
            if let Some(worker) = &self.cache_worker {
                worker.cancel.store(true, Ordering::Release);
                self.cache.phase = CachePhase::Cancelling;
            }
            return;
        }
        if self.cache_worker.is_some() {
            return;
        }
        if matches!(action, CacheAction::Clear | CacheAction::Retry) {
            self.failed.borrow_mut().clear();
            self.recovery.borrow_mut().clear();
            self.visual_cause.borrow_mut().take();
        }
        self.cache_operation = match action {
            CacheAction::Clear => CacheOperation::Clear,
            CacheAction::Retry if self.cache.phase == CachePhase::Failed => self.cache_operation,
            _ => CacheOperation::Inspect,
        };
        let snapshots =
            (|| {
                let access = self.access.as_ref().ok_or(engine::Error::Edit(
                    engine::EditError::Unsupported("media cache unavailable"),
                ))?;
                let access = access.try_lock().map_err(|error| match error {
                    std::sync::TryLockError::WouldBlock => {
                        engine::Error::Runtime(engine::RuntimeError::CacheBusy)
                    }
                    std::sync::TryLockError::Poisoned(_) => engine::RuntimeError::Poisoned.into(),
                })?;
                access.cache_maintenance()
            })();
        let snapshots = match snapshots {
            Ok(snapshots) => snapshots,
            Err(error) => {
                self.fail_cache(error);
                return;
            }
        };
        let operation = self.cache_operation;
        // Snapshot acquisition above has released MediaAccess before worker IO.
        let spawned = CacheWorker::spawn(move |flag| {
            let mut report = CacheMaintenanceReport::default();
            for snapshot in snapshots {
                report.merge(snapshot.run(operation, || flag.load(Ordering::Acquire)));
            }
            report
        });
        match spawned {
            Ok(worker) => {
                self.cache.cause = None;
                self.cache_worker = Some(worker);
                self.cache.phase = if operation == CacheOperation::Clear {
                    CachePhase::Clearing
                } else {
                    CachePhase::Inspecting
                };
            }
            Err(error) => self.fail_cache(error.into()),
        }
    }
    fn fail_cache(&mut self, error: engine::Error) {
        // Keep the last measured size/age; a failed admission measured no zero.
        self.cache.phase = CachePhase::Failed;
        self.cache.cause = Some(Arc::new(error));
    }
    fn finish_cache(&mut self, report: CacheMaintenanceReport) {
        self.cache.cause = None;
        self.cache.phase = if report.cause.is_some() {
            CachePhase::Failed
        } else {
            CachePhase::Idle
        };
        self.cache.report = Some(Arc::new(report));
        self.cache.checked = Some(std::time::Instant::now());
    }
    /// Only an actual typed corruption permits one regeneration per visual key.
    fn display_failed(&self, error: engine::Error, request: VisualRequest) -> bool {
        let corrupt = matches!(
            error,
            engine::Error::Runtime(engine::RuntimeError::CacheCorrupt)
                | engine::Error::Artifacts(engine::ArtifactError::Runtime(
                    engine::RuntimeError::CacheCorrupt
                ))
        );
        self.visual_cause
            .borrow_mut()
            .get_or_insert_with(|| Arc::new(error));
        if corrupt && self.pending.borrow().contains_key(&request.key()) {
            return true;
        }
        let admitted = self.recovery.borrow().get(&request.key()).copied();
        let allowed =
            admitted == Some(false) || (admitted.is_none() && self.recovery.borrow().len() < 256);
        if corrupt && allowed {
            self.recovery
                .borrow_mut()
                .entry(request.key())
                .or_insert(false);
            self.enqueue(request);
            true
        } else {
            self.failed.borrow_mut().insert(request.key());
            false
        }
    }

    /// Run one display query with the shared caches; `None` while detached.
    fn query<T>(&self, read: impl FnOnce(&mut engine::control::MediaAccess) -> T) -> Option<T> {
        let access = self.access.as_ref()?;
        let mut access = access.lock().ok()?;
        Some(read(&mut access))
    }

    fn texture(
        &self,
        ctx: &egui::Context,
        asset: &media::SourceAsset,
        at: Tick,
        image: &engine::thumbnails::RetainedThumbnail,
    ) -> egui::TextureHandle {
        let key = (asset.id(), at, asset.content_hash().0);
        let mut textures = self.textures.borrow_mut();
        if let Some(texture) = textures.get(&key) {
            return texture.clone();
        }
        if textures.len() >= MAX_TEXTURES
            && let Some(oldest) = textures.keys().next().copied()
        {
            textures.remove(&oldest);
        }
        let pixels = egui::ColorImage::from_rgba_unmultiplied(
            [image.width as usize, image.height as usize],
            &image.pixels,
        );
        let texture = ctx.load_texture(
            format!("timeline.strip.{}.{}", asset.id().0, at.0),
            pixels,
            egui::TextureOptions::LINEAR,
        );
        textures.insert(key, texture.clone());
        texture
    }

    /// Queue one generation command per cache family; warm frames ask nothing.
    fn enqueue(&self, request: VisualRequest) {
        let key = request.key();
        let mut pending = self.pending.borrow_mut();
        if pending.contains_key(&key) || self.failed.borrow().contains(&key) {
            return;
        }
        pending.insert(key, request.clone());
        self.queued.borrow_mut().push(request);
    }

    /// Submit one generation command the panel requested.
    pub(in crate::editor) fn generate(
        &mut self,
        session: &mut engine::Session,
        request: VisualRequest,
    ) -> crate::Result<()> {
        let key = request.key();
        if session.transaction_active() {
            // A preview gesture owns the session; the next paint asks again.
            self.pending.borrow_mut().remove(&key);
            return Ok(());
        }
        let (command, params) = request.command();
        let response = match self.control(session, command, params) {
            Ok(response) => response,
            Err(error) => {
                self.pending.borrow_mut().remove(&key);
                return Err(error);
            }
        };
        match response.value {
            engine::control::ResponseValue::Result(
                engine::control::ControlResult::JobAccepted { job, .. },
            ) => {
                if let Some(admitted) = self.recovery.borrow_mut().get_mut(&key) {
                    *admitted = true;
                }
                self.jobs.insert(job, request);
                Ok(())
            }
            _ => {
                self.pending.borrow_mut().remove(&key);
                self.failed.borrow_mut().insert(key);
                Err(crate::Error::Invalid("clip visual generation refused"))
            }
        }
    }

    fn control(
        &mut self,
        session: &mut engine::Session,
        command: &str,
        params: serde_json::Value,
    ) -> crate::Result<engine::control::Response> {
        let Some(access) = &self.access else {
            return Err(crate::Error::Invalid("media cache unavailable"));
        };
        if self.runner.is_none() {
            let runner = engine::jobs::JobRunner::new(
                engine::jobs::RunnerConfig {
                    concurrency: [1, 1, 1],
                    queue: 32,
                    retained: 32,
                    network_attempts: 1,
                    backoff: std::time::Duration::from_millis(100),
                    progress_interval: std::time::Duration::from_millis(50),
                },
                self.budget.clone(),
                std::sync::Arc::new(engine::jobs::SystemClock::default()),
            )?;
            self.runner = Some(runner);
        }
        let runner = self
            .runner
            .as_mut()
            .ok_or(crate::Error::Invalid("visual job runner"))?;
        self.request_id = self.request_id.saturating_add(1);
        let id = self.request_id;
        let request = engine::control::Request {
            api_version: engine::control::API_VERSION,
            registry_hash: Some(engine::control::registry_hash().into()),
            session_id: session.session_id().into(),
            request_id: format!("timeline-visual-{id}"),
            idempotency_key: Some(format!("timeline-visual-{id}")),
            base_rev: Some(session.revision()),
            command: command.to_owned(),
            params,
        };
        let response = session.control_request(
            request,
            engine::control::Caller {
                principal: "vibecut-timeline",
                external: false,
                policy: &EditorPolicy,
                fonts: None,
                sounds: None,
                media: Some(access.as_ref()),
                chapters: None,
                captions: None,
            },
            runner,
        )?;
        Ok(response)
    }

    /// Advance generation jobs and publish finished outputs through `job_status`.
    pub(in crate::editor) fn tick(&mut self, session: &mut engine::Session) -> crate::Result<()> {
        if self
            .cache_worker
            .as_ref()
            .is_some_and(|worker| worker.handle.is_finished())
            && let Some(worker) = self.cache_worker.take()
        {
            let report = worker
                .handle
                .join()
                .unwrap_or_else(|_| CacheMaintenanceReport {
                    cause: Some(engine::RuntimeError::WorkerPanicked.into()),
                    ..Default::default()
                });
            self.finish_cache(report);
        }
        if let Some(runner) = &mut self.runner {
            runner.tick()?;
        }
        let ids: Vec<engine::jobs::JobId> = self.jobs.keys().copied().collect();
        for id in ids {
            let state = self
                .runner
                .as_ref()
                .and_then(|runner| runner.snapshot(id))
                .map(|snapshot| snapshot.state);
            let Some(state) = state else {
                self.settle(id, None);
                continue;
            };
            if !matches!(
                state,
                engine::jobs::JobState::Completed
                    | engine::jobs::JobState::Cancelled
                    | engine::jobs::JobState::Failed
            ) {
                continue;
            }
            let result = self.status(session, id);
            self.settle(id, Some((state, result)));
        }
        Ok(())
    }

    fn status(
        &mut self,
        session: &mut engine::Session,
        job: engine::jobs::JobId,
    ) -> crate::Result<()> {
        let response = self.control(session, "job_status", serde_json::json!({"job": job}))?;
        match response.value {
            engine::control::ResponseValue::Result(engine::control::ControlResult::Job {
                ..
            }) => Ok(()),
            _ => Err(crate::Error::Invalid("clip visual job status")),
        }
    }

    fn settle(
        &mut self,
        job: engine::jobs::JobId,
        outcome: Option<(engine::jobs::JobState, crate::Result<()>)>,
    ) {
        let Some(request) = self.jobs.remove(&job) else {
            return;
        };
        self.pending.borrow_mut().remove(&request.key());
        if let Some((state, result)) = outcome
            && (state == engine::jobs::JobState::Failed || result.is_err())
        {
            self.failed.borrow_mut().insert(request.key());
        }
    }
}

impl VisualQuery for Media {
    fn strip(
        &self,
        ctx: &egui::Context,
        asset: &media::SourceAsset,
        item: &project::Item,
        visible: time::TimeRange,
        pixels_per_second: f64,
    ) -> StripState {
        let key = VisualKey::Thumbnails(item.id);
        if self.failed.borrow().contains(&key) {
            return StripState::Failed;
        }
        let Some(state) = self.query(|access| {
            access.thumbnail_strip_display(asset, item, visible, pixels_per_second)
        }) else {
            return StripState::Pending;
        };
        let error = match state {
            Ok(engine::thumbnail_strip::ThumbnailStripState::Ready(display)) => {
                return StripState::Ready(
                    display
                        .tiles()
                        .iter()
                        .map(|tile| StripTile {
                            output: tile.output,
                            texture: self.texture(ctx, asset, tile.at, &tile.image),
                        })
                        .collect(),
                );
            }
            Ok(engine::thumbnail_strip::ThumbnailStripState::Pending) => {
                self.enqueue(VisualRequest::Thumbnails {
                    item: item.id,
                    visible,
                    pixels_per_second,
                });
                return StripState::Pending;
            }
            Ok(engine::thumbnail_strip::ThumbnailStripState::Failed(error)) => error.into(),
            Err(error) => error,
            Ok(engine::thumbnail_strip::ThumbnailStripState::Unsupported) => {
                return StripState::Unsupported;
            }
        };
        if self.display_failed(
            error,
            VisualRequest::Thumbnails {
                item: item.id,
                visible,
                pixels_per_second,
            },
        ) {
            StripState::Pending
        } else {
            StripState::Failed
        }
    }

    fn waveform(
        &self,
        asset: &media::SourceAsset,
        item: &project::Item,
        visible: time::TimeRange,
        columns: usize,
    ) -> WaveState {
        let key = VisualKey::Waveform(asset.id());
        if self.failed.borrow().contains(&key) {
            return WaveState::Failed;
        }
        let Some(state) =
            self.query(|access| access.waveform_display(asset, item, visible, columns))
        else {
            return WaveState::Pending;
        };
        let error = match state {
            Ok(engine::waveform::WaveformState::Ready(display)) => {
                return WaveState::Ready(
                    display
                        .bars()
                        .iter()
                        .map(|bar| WaveBar {
                            min: bar.min,
                            max: bar.max,
                            over: bar.over,
                        })
                        .collect(),
                );
            }
            Ok(engine::waveform::WaveformState::Pending) => {
                self.enqueue(VisualRequest::Waveform { asset: asset.id() });
                return WaveState::Pending;
            }
            Ok(engine::waveform::WaveformState::Failed(error)) => error.into(),
            Err(error) => error,
            Ok(engine::waveform::WaveformState::Unsupported) => return WaveState::Unsupported,
        };
        if self.display_failed(error, VisualRequest::Waveform { asset: asset.id() }) {
            WaveState::Pending
        } else {
            WaveState::Failed
        }
    }

    fn take_requests(&self) -> Vec<VisualRequest> {
        std::mem::take(&mut *self.queued.borrow_mut())
    }
}

/// The warning ink for an over-full-scale waveform tip.
pub(in crate::editor) fn clip_tip(dark: bool) -> egui::Color32 {
    theme::warning(dark)
}

impl Drop for Media {
    fn drop(&mut self) {
        if let Some(worker) = self.cache_worker.take() {
            worker.cancel.store(true, Ordering::Release);
            let _ = worker.handle.join();
        }
    }
}

#[cfg(test)]
#[test]
#[allow(clippy::unwrap_used, clippy::panic)]
fn cache_worker_cancel_and_drop_join_and_panics_stay_typed() {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    let budget = engine::ByteBudget::new(8192).unwrap();
    let mut media = Media::new(budget.clone());
    let (ready, started) = mpsc::sync_channel(1);
    let (release, gate) = mpsc::sync_channel(1);
    media.cache_worker = Some(
        CacheWorker::spawn(move |cancel| {
            ready.send(()).unwrap();
            gate.recv().unwrap();
            assert!(cancel.load(Ordering::Acquire));
            CacheMaintenanceReport {
                removed_bytes: 100,
                cause: Some(engine::RuntimeError::Cancelled),
                ..Default::default()
            }
        })
        .unwrap(),
    );
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    media.request_cache(CacheAction::Cancel);
    assert_eq!(media.cache_ui().phase, CachePhase::Cancelling);
    release.send(()).unwrap();
    let mut session = engine::Session::new(
        project::Project::empty(project::ProjectId(1), "worker".into()).unwrap(),
        10,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while media.cache_worker.is_some() {
        media.tick(&mut session).unwrap();
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let report = media.cache_ui().report.unwrap();
    assert_eq!(report.removed_bytes, 100);
    assert!(matches!(
        report.cause,
        Some(engine::RuntimeError::Cancelled)
    ));
    let worker = CacheWorker::spawn(|_| panic!("injected worker unwind")).unwrap();
    let report = worker.handle.join().unwrap();
    assert!(matches!(
        report.cause,
        Some(engine::RuntimeError::WorkerPanicked)
    ));
    let (done, joined) = mpsc::sync_channel(1);
    media.cache_worker = Some(
        CacheWorker::spawn(move |cancel| {
            while !cancel.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            done.send(()).unwrap();
            CacheMaintenanceReport::default()
        })
        .unwrap(),
    );
    drop(media);
    joined.try_recv().unwrap();
}
