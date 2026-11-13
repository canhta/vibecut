//! App-owned retained frames and device clock, scheduled and accounted by engine.
use crate::{Error, Result, egui};
use engine::cache::{ArtifactKind, CacheKey, KeyInputs};
use engine::jobs::{
    CancelToken, JobBytes, JobKind, JobRunner, JobSpec, JobState, ResourceClass, RunnerConfig,
    SystemClock,
};
use engine::{ByteBudget, ByteLease, ByteOwner, Session};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::Duration,
};
use time::{Rounding, TICKS_PER_SECOND, Tick};

const RING_FRAMES: usize = 65536;
const BLOCK: usize = 1024;
const MAX_FRAMES: usize = 4;

/// One transport request from a player's controls or keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// Toggle play; starting at the end restarts from zero.
    Play,
    Pause,
    /// Absolute position, clamped to the timeline.
    Seek(Tick),
    BackSecond,
    ForwardSecond,
    FrameBack,
    FrameForward,
    Start,
    End,
}

/// Render's source seam is injected; export uses `Files`, the same FileSources
/// a headless export reads. Only preview ever receives `Proxied`.
pub trait Sources: Send + Sync + 'static {
    fn create(&self) -> Box<dyn render::SourceProvider>;
    /// The proxy choice and files this factory resolves, if it resolves any.
    fn proxies(&self) -> Option<&engine::proxies::ProxyIndex> {
        None
    }
}
pub struct Files;
impl Sources for Files {
    fn create(&self) -> Box<dyn render::SourceProvider> {
        Box::new(render::FileSources::default())
    }
}
/// Preview sources that read cached proxies while the project's choice is on.
pub struct Proxied(pub engine::proxies::ProxyIndex);
impl Sources for Proxied {
    fn create(&self) -> Box<dyn render::SourceProvider> {
        Box::new(engine::proxies::ProxySources::new(self.0.clone()))
    }
    fn proxies(&self) -> Option<&engine::proxies::ProxyIndex> {
        Some(&self.0)
    }
}
struct Slot {
    epoch: AtomicU64,
    index: AtomicU64,
    left: AtomicU32,
    right: AtomicU32,
}
/// The only object accessed by the real-time callback: fixed storage and atomics.
/// `fill` never allocates, locks, decodes, logs or performs IO.
pub struct AudioPort {
    slots: Vec<Slot>,
    submitted: AtomicU64,
    audible: AtomicU64,
    epoch: AtomicU64,
    playing: AtomicBool,
    failed: AtomicBool,
    underruns: AtomicU64,
    /// The ring's bytes; a successor port over the same budget shares them.
    lease: Arc<ByteLease>,
}
impl AudioPort {
    fn new(budget: &ByteBudget) -> Result<Self> {
        let lease = budget.reserve(
            ByteOwner::DecodedFrames,
            (RING_FRAMES * std::mem::size_of::<Slot>()) as u64,
        )?;
        Ok(Self::with_lease(Arc::new(lease)))
    }
    fn with_lease(lease: Arc<ByteLease>) -> Self {
        Self {
            slots: (0..RING_FRAMES)
                .map(|_| Slot {
                    epoch: AtomicU64::new(0),
                    index: AtomicU64::new(0),
                    left: AtomicU32::new(0),
                    right: AtomicU32::new(0),
                })
                .collect(),
            submitted: AtomicU64::new(0),
            audible: AtomicU64::new(0),
            epoch: AtomicU64::new(1),
            playing: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            underruns: AtomicU64::new(0),
            lease,
        }
    }
    /// Stereo f32 at the project's render-owned sample rate. `latency_frames`
    /// comes from the device's callback/playback timestamp difference.
    pub fn fill(&self, output: &mut [f32], latency_frames: u64) {
        output.fill(0.0);
        let start = self.submitted.load(Ordering::Acquire);
        let epoch = self.epoch.load(Ordering::Acquire);
        let playing = self.playing.load(Ordering::Acquire);
        let mut missing = 0;
        for (n, frame) in output.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let index = start.saturating_add(n as u64);
            if playing && let Some(slot) = self.slots.get(index as usize % RING_FRAMES) {
                let ready = slot.epoch.load(Ordering::Acquire) == epoch
                    && slot.index.load(Ordering::Relaxed) == index;
                let left = f32::from_bits(slot.left.load(Ordering::Relaxed));
                let right = f32::from_bits(slot.right.load(Ordering::Relaxed));
                if ready && slot.epoch.load(Ordering::Acquire) == epoch {
                    if let Some(x) = frame.get_mut(0) {
                        *x = left;
                    }
                    if let Some(x) = frame.get_mut(1) {
                        *x = right;
                    }
                } else {
                    missing += 1;
                }
            }
        }
        // A concurrent seek/pause cannot leak an already-copied old buffer.
        if self.epoch.load(Ordering::Acquire) != epoch || !self.playing.load(Ordering::Acquire) {
            output.fill(0.0);
        }
        self.underruns.fetch_add(missing, Ordering::Relaxed);
        self.audible
            .store(start.saturating_sub(latency_frames), Ordering::Release);
        self.submitted.store(
            start.saturating_add((output.len() / 2) as u64),
            Ordering::Release,
        );
    }
    pub fn fail(&self) {
        self.failed.store(true, Ordering::Release);
    }
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }
    pub fn underrun_frames(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }
}
/// One transient render-state owner shared by editor preview and export.
/// Each consumer freezes settings against the same immutable project snapshot;
/// removed items are filtered without losing their placements for undo/redo.
#[derive(Default)]
pub struct RenderState(Mutex<Option<Arc<render::RenderSettings>>>);
impl RenderState {
    pub(crate) fn settings(&self, project: &project::Project) -> Result<render::RenderSettings> {
        let retained = self.0.lock().map_err(|_| Error::Poisoned)?.clone();
        let mut settings = retained
            .as_deref()
            .cloned()
            .unwrap_or_else(|| render::RenderSettings::from_project(project));
        settings.placements.retain(|id, _| {
            project
                .data()
                .scene()
                .timeline
                .iter()
                .any(|track| track.items.iter().any(|item| item.id == *id))
        });
        Ok(settings)
    }
    /// Whether a transient render placement replaces this item's saved box.
    pub(crate) fn overrides_placement(&self, item: project::ItemId) -> bool {
        self.0
            .lock()
            .map(|state| {
                state
                    .as_deref()
                    .is_some_and(|settings| settings.placements.contains_key(&item))
            })
            .unwrap_or(true)
    }
}
#[derive(Clone)]
struct Request {
    render_state: Arc<RenderState>,
    effects: Option<Arc<dyn render::effects::EffectCompositor>>,
    epoch: u64,
    generation: u64,
    project: Arc<project::Project>,
    start: Tick,
    origin: u64,
    playing: bool,
    /// Transient preview-axis time; `None` follows the transport.
    hover: Option<Tick>,
    /// Loop `[start, end)` in ticks; `None` plays through to the timeline end.
    loop_range: Option<(Tick, Tick)>,
}
impl Request {
    /// The loop range clamped to this snapshot's duration; `None` when empty.
    fn loop_bounds(&self) -> Option<(Tick, Tick)> {
        let (start, end) = self.loop_range?;
        let end = end.min(self.project.duration());
        (start >= Tick::ZERO && start < end).then_some((start, end))
    }
    /// The tick the preview shows: the pointer time while the axis holds one.
    fn display_at(&self, counter: u64) -> Result<Tick> {
        match self.hover {
            Some(at) => Ok(at),
            None => self.at(counter),
        }
    }
    fn at(&self, counter: u64) -> Result<Tick> {
        let delta = if self.playing {
            counter.saturating_sub(self.origin)
        } else {
            0
        };
        let delta = i64::try_from(delta).map_err(|_| Error::Invalid("device clock overflow"))?;
        let at = self
            .start
            .checked_add(Tick::from_units(
                delta,
                i64::from(self.project.data().settings.audio_sample_rate),
                Rounding::Nearest,
            )?)
            .ok_or(Error::Invalid("transport overflow"))?;
        Ok(match self.loop_bounds() {
            Some((start, end)) if self.playing => Tick(wrap_units(at.0, start.0, end.0)),
            _ => at.min(self.project.duration()),
        })
    }
}
pub struct Frame {
    pub generation: u64,
    pub at: Tick,
    pub image: egui::ColorImage,
    /// Covers CPU ColorImage, egui upload and GPU texture retention. Native driver
    /// staging/allocator overhead is measured separately, never claimed as capped RSS.
    _lease: ByteLease,
}
struct Output {
    frames: VecDeque<Arc<Frame>>,
    error: Option<Arc<Error>>,
    audio_error: Option<Arc<Error>>,
}
enum Workers {
    Running,
    Failed(Arc<dyn engine::Failure + 'static>),
    Restarting,
}
pub struct Playback {
    runner: JobRunner,
    cancels: Vec<CancelToken>,
    jobs: Vec<engine::jobs::JobId>,
    workers: Workers,
    spec: JobSpec,
    sources: Arc<dyn Sources>,
    request: Arc<Mutex<Request>>,
    output: Arc<Mutex<Output>>,
    port: Arc<AudioPort>,
    preview_generation: Arc<AtomicU64>,
    budget: ByteBudget,
    shown: Option<Arc<Frame>>,
    last_callback: (u64, std::time::Instant),
    device_available: bool,
}
impl Playback {
    /// Attach the desktop device adapter before the first visible preview.
    pub fn set_effects(
        &mut self,
        effects: Arc<dyn render::effects::EffectCompositor>,
    ) -> Result<()> {
        self.request.lock().map_err(|_| Error::Poisoned)?.effects = Some(effects);
        let request = read_request(&self.request)?;
        self.seek(request.start, request.playing, request.project)
    }
    /// Use the same transient placement/crop settings as the accepted export.
    /// Render validates geometry; a change invalidates every queued preview frame.
    pub fn set_render_settings(&mut self, settings: render::RenderSettings) -> Result<()> {
        let request = read_request(&self.request)?;
        render::Renderer::new(&request.project, settings.clone())?;
        *request.render_state.0.lock().map_err(|_| Error::Poisoned)? = Some(Arc::new(settings));
        self.seek(self.position()?, false, request.project)
    }
    /// The shared placement/crop render state the preview and an export job use.
    pub fn render_state(&self) -> Result<Arc<RenderState>> {
        Ok(read_request(&self.request)?.render_state)
    }
    /// Whether a transient placement replaces this item's saved box, so saved
    /// geometry commands must refuse rather than align a different rectangle.
    pub fn placement_overridden(&self, item: project::ItemId) -> bool {
        read_request(&self.request)
            .map(|request| request.render_state.overrides_placement(item))
            .unwrap_or(true)
    }
    pub fn new(session: &Session, budget: ByteBudget, sources: Arc<dyn Sources>) -> Result<Self> {
        let port = AudioPort::new(&budget)?;
        Self::over(session, budget, sources, port)
    }
    /// A fresh preview of `session`, a copy of the project this preview shows, over the
    /// same budget. Its new audio ring shares the bytes this ring holds, so the copy
    /// opens under any memory pressure; this preview must be dropped once the copy
    /// replaces it.
    pub fn successor(&self, session: &Session, sources: Arc<dyn Sources>) -> Result<Self> {
        let port = AudioPort::with_lease(self.port.lease.clone());
        Self::over(session, self.budget.clone(), sources, port)
    }
    fn over(
        session: &Session,
        budget: ByteBudget,
        sources: Arc<dyn Sources>,
        port: AudioPort,
    ) -> Result<Self> {
        if let Some(index) = sources.proxies() {
            index.set_enabled(session.metadata().proxy_preview);
        }
        let port = Arc::new(port);
        let request = Arc::new(Mutex::new(Request {
            render_state: Arc::new(RenderState::default()),
            effects: None,
            epoch: 1,
            generation: 1,
            project: session.project().clone(),
            start: Tick::ZERO,
            origin: 0,
            playing: false,
            hover: None,
            loop_range: None,
        }));
        let output = Arc::new(Mutex::new(Output {
            frames: VecDeque::new(),
            error: None,
            audio_error: None,
        }));
        let config = RunnerConfig {
            concurrency: [2, 1, 1],
            queue: 4,
            retained: 4,
            network_attempts: 1,
            backoff: Duration::from_millis(10),
            progress_interval: Duration::from_millis(20),
        };
        let runner = JobRunner::new(config, budget.clone(), Arc::new(SystemClock::default()))?;
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::Preview,
            source: [0; 32],
            provider: "desktop",
            model: "preview",
            version: "1",
            settings: &[],
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        let spec = JobSpec {
            kind: JobKind::Preview,
            class: ResourceClass::Cpu,
            input: key,
            bytes: JobBytes::default(),
        };
        let mut playback = Self {
            runner,
            cancels: Vec::new(),
            jobs: Vec::new(),
            workers: Workers::Running,
            spec,
            sources,
            request,
            output,
            port,
            preview_generation: Arc::new(AtomicU64::new(1)),
            budget,
            shown: None,
            last_callback: (0, std::time::Instant::now()),
            device_available: false,
        };
        playback.submit_workers()?;
        playback.runner.tick()?;
        Ok(playback)
    }
    // Called only at construction or after every obsolete job is terminal/joined.
    fn submit_workers(&mut self) -> Result<()> {
        self.cancels.clear();
        self.jobs.clear();
        for audio in [true, false] {
            let preview_generation = self.preview_generation.clone();
            let (request, output, port, sources, budget) = (
                self.request.clone(),
                self.output.clone(),
                self.port.clone(),
                self.sources.clone(),
                self.budget.clone(),
            );
            let submitted = self
                .runner
                .submit::<engine::control::Error>(self.spec, move |hooks| {
                    while !hooks.cancelled() {
                        let generation = if audio {
                            &port.epoch
                        } else {
                            &preview_generation
                        };
                        let epoch = generation.load(Ordering::Acquire);
                        let result = if audio {
                            audio_loop(&request, &port, sources.as_ref(), hooks)
                        } else {
                            video_loop(
                                &request,
                                &output,
                                &port,
                                &preview_generation,
                                &budget,
                                sources.as_ref(),
                                hooks,
                            )
                        };
                        if let Err(error) = result
                            && !hooks.cancelled()
                        {
                            if let Ok(mut out) = output.lock()
                                && generation.load(Ordering::Acquire) == epoch
                            {
                                if audio {
                                    out.audio_error = Some(Arc::new(error));
                                } else {
                                    out.error = Some(Arc::new(error));
                                }
                            }
                            while !hooks.cancelled() && generation.load(Ordering::Acquire) == epoch
                            {
                                std::thread::sleep(Duration::from_millis(2));
                            }
                        }
                    }
                    Ok(Vec::new())
                });
            match submitted {
                Ok((job, cancel)) => {
                    self.cancels.push(cancel);
                    self.jobs.push(job);
                }
                Err(error) => {
                    for cancel in &self.cancels {
                        cancel.cancel();
                    }
                    let error = Arc::new(error);
                    self.workers = Workers::Failed(error.clone());
                    return Err(Error::Worker(error));
                }
            }
        }
        self.workers = Workers::Running;
        Ok(())
    }
    fn poll_workers(&mut self) -> Result<()> {
        // tick joins only finished handles; Retry must never wait on decoding IO.
        self.runner.tick()?;
        if let Workers::Failed(error) = &self.workers {
            return Err(Error::Worker(error.clone()));
        }
        if matches!(self.workers, Workers::Restarting) {
            let stopped = self.jobs.iter().all(|job| {
                self.runner.snapshot(*job).is_some_and(|snapshot| {
                    matches!(
                        snapshot.state,
                        JobState::Completed | JobState::Cancelled | JobState::Failed
                    )
                })
            });
            if stopped {
                self.submit_workers()?;
                self.runner.tick()?;
            }
            return Ok(());
        }
        let error = self.jobs.iter().find_map(|job| {
            self.runner
                .snapshot(*job)
                .and_then(|snapshot| snapshot.error)
        });
        if let Some(error) = error {
            let request = read_request(&self.request)?;
            self.seek(self.position()?, false, request.project)?;
            for cancel in &self.cancels {
                cancel.cancel();
            }
            self.workers = Workers::Failed(error.clone());
            return Err(Error::Worker(error));
        }
        Ok(())
    }
    pub fn audio_port(&self) -> Arc<AudioPort> {
        self.port.clone()
    }
    pub fn set_device_available(&mut self, available: bool) {
        self.device_available = available;
    }
    pub fn device_available(&self) -> bool {
        self.device_available && !self.port.failed()
    }
    /// Device and preview jobs are available; the caller also checks timeline content.
    pub fn can_play(&self) -> bool {
        self.device_available() && matches!(self.workers, Workers::Running)
    }
    /// The render source seam this playback's workers read through; `export.clips` lends it too.
    pub fn export_sources(
        &self,
    ) -> impl Fn() -> Box<dyn render::SourceProvider> + Send + Sync + 'static {
        let sources = self.sources.clone();
        move || sources.create()
    }
    pub fn budget(&self) -> &ByteBudget {
        &self.budget
    }
    /// The proxies preview reads; `None` for sources that never resolve any.
    pub fn proxies(&self) -> Option<&engine::proxies::ProxyIndex> {
        self.sources.proxies()
    }
    /// Turn proxy preview on or off. The shown frame was read from the other source
    /// choice, so the current frame is requested again: a cleared proxy then falls
    /// back to the source with its typed reason instead of waiting for the next seek.
    pub fn set_proxy_preview(&mut self, on: bool) -> Result<()> {
        let Some(index) = self.sources.proxies() else {
            return Ok(());
        };
        if index.enabled() == on {
            return Ok(());
        }
        index.set_enabled(on);
        let request = read_request(&self.request)?;
        self.seek(self.position()?, request.playing, request.project)
    }
    pub fn position(&self) -> Result<Tick> {
        self.request
            .lock()
            .map_err(|_| Error::Poisoned)?
            .at(self.port.audible.load(Ordering::Acquire))
    }
    /// The preview-axis time the next frame renders, if one is held.
    pub fn hover(&self) -> Option<Tick> {
        self.request.lock().ok().and_then(|request| request.hover)
    }
    /// The tick the player shows: the hover time while one is held, otherwise
    /// the transport position. The transport itself never moves.
    pub fn display_position(&self) -> Result<Tick> {
        self.request
            .lock()
            .map_err(|_| Error::Poisoned)?
            .display_at(self.port.audible.load(Ordering::Acquire))
    }
    pub fn playing(&self) -> bool {
        self.port.playing.load(Ordering::Acquire)
    }
    /// Preview generation, bumped by seek and pointer requests. Frames and
    /// uploads carry it; the independent audio epoch changes only with transport.
    pub fn generation(&self) -> u64 {
        self.preview_generation.load(Ordering::Acquire)
    }
    /// Apply one transport request against `project`, the immutable snapshot the
    /// preview renders. Play toggles, an end-position play restarts from zero,
    /// and every seek stays inside the timeline.
    pub fn transport(
        &mut self,
        transport: Transport,
        project: Arc<project::Project>,
    ) -> Result<()> {
        let duration = project.duration();
        let at = self.position()?.clamp(Tick::ZERO, duration);
        let frame = project
            .data()
            .canvas
            .fps
            .frame_duration()
            .ok()
            .filter(|frame| *frame > Tick::ZERO)
            .unwrap_or(Tick(1));
        let (at, playing) = match transport {
            Transport::Play => (
                if at >= duration { Tick::ZERO } else { at },
                !self.playing(),
            ),
            Transport::Pause => (at, false),
            Transport::Seek(at) => (at.clamp(Tick::ZERO, duration), self.playing()),
            Transport::BackSecond => (
                (at - Tick(TICKS_PER_SECOND)).max(Tick::ZERO),
                self.playing(),
            ),
            Transport::ForwardSecond => {
                ((at + Tick(TICKS_PER_SECOND)).min(duration), self.playing())
            }
            Transport::FrameBack => ((at - frame).clamp(Tick::ZERO, duration), self.playing()),
            Transport::FrameForward => ((at + frame).clamp(Tick::ZERO, duration), self.playing()),
            Transport::Start => (Tick::ZERO, self.playing()),
            Transport::End => (duration, self.playing()),
        };
        self.seek(at, playing, project)
    }
    /// The stored loop range, `[start, end)` ticks, before clamping to the timeline.
    pub fn loop_range(&self) -> Result<Option<(Tick, Tick)>> {
        Ok(read_request(&self.request)?.loop_range)
    }
    /// Loop `[start, end)`, or the whole timeline with `None`. A paused transport keeps
    /// its position. A playing one restarts its audio epoch at the same position, so only
    /// a changed range re-seeks. Ranges must satisfy `0 <= start < end <= duration`.
    pub fn set_loop(
        &mut self,
        range: Option<(Tick, Tick)>,
        project: Arc<project::Project>,
    ) -> Result<()> {
        if let Some((start, end)) = range
            && (start < Tick::ZERO || start >= end || end > project.duration())
        {
            return Err(Error::Invalid("loop range"));
        }
        if read_request(&self.request)?.loop_range == range {
            return Ok(());
        }
        self.request.lock().map_err(|_| Error::Poisoned)?.loop_range = range;
        self.seek(self.position()?, self.playing(), project)
    }
    pub fn seek(&mut self, at: Tick, playing: bool, project: Arc<project::Project>) -> Result<()> {
        if at < Tick::ZERO || at > project.duration() {
            return Err(Error::Invalid("seek outside timeline"));
        }
        if playing && !self.device_available() {
            return Err(Error::PreviewUnavailable("audio device"));
        }
        if playing && !matches!(self.workers, Workers::Running) {
            return Err(Error::PreviewUnavailable("preview workers"));
        }
        let mut request = self.request.lock().map_err(|_| Error::Poisoned)?;
        let epoch = request
            .epoch
            .checked_add(1)
            .ok_or(Error::Invalid("audio generation exhausted"))?;
        let generation = request
            .generation
            .checked_add(1)
            .ok_or(Error::Invalid("preview generation exhausted"))?;
        // Invalidation and publication share this lock: a new pinned frame
        // cannot publish between advancing its generation and clearing output.
        let mut output = self.output.lock().map_err(|_| Error::Poisoned)?;
        self.port.playing.store(false, Ordering::Release);
        *request = Request {
            render_state: request.render_state.clone(),
            effects: request.effects.clone(),
            epoch,
            generation,
            project,
            start: at,
            origin: self.port.submitted.load(Ordering::Acquire),
            playing,
            hover: None,
            loop_range: request.loop_range,
        };
        self.port.epoch.store(epoch, Ordering::Release);
        self.preview_generation.store(generation, Ordering::Release);
        self.port.playing.store(playing, Ordering::Release);
        self.shown = None;
        output.frames.clear();
        output.error = None;
        output.audio_error = None;
        self.last_callback = (
            self.port.submitted.load(Ordering::Acquire),
            std::time::Instant::now(),
        );
        Ok(())
    }
    /// Render at a transient pointer time without moving the transport. Every
    /// change fenced by a new generation, so only the latest pointer time is
    /// shown and a superseded frame is dropped instead of queued.
    pub fn set_hover(&mut self, at: Option<Tick>) -> Result<()> {
        let mut request = self.request.lock().map_err(|_| Error::Poisoned)?;
        let at = at
            .map(|at| {
                request
                    .project
                    .data()
                    .canvas
                    .fps
                    .snap_nearest(at.max(Tick::ZERO))
            })
            .transpose()?;
        if request.hover == at {
            return Ok(());
        }
        let generation = request
            .generation
            .checked_add(1)
            .ok_or(Error::Invalid("preview generation exhausted"))?;
        let mut output = self.output.lock().map_err(|_| Error::Poisoned)?;
        request.hover = at;
        request.generation = generation;
        self.preview_generation.store(generation, Ordering::Release);
        self.shown = None;
        output.frames.clear();
        output.error = None;
        Ok(())
    }
    /// Healthy clearing preserves transport. After worker failure, Retry keeps it
    /// paused and replaces both jobs once cancellation has finished, without waiting.
    pub fn clear(&mut self) -> Result<()> {
        match self.poll_workers() {
            Err(Error::Worker(_)) if matches!(self.workers, Workers::Failed(_)) => {
                self.workers = Workers::Restarting;
                return Ok(());
            }
            result => result?,
        }
        if matches!(self.workers, Workers::Restarting) {
            return Ok(());
        }
        let request = self.request.lock().map_err(|_| Error::Poisoned)?.clone();
        self.seek(self.position()?, request.playing, request.project)
    }
    pub fn error(&self) -> Option<Arc<Error>> {
        if let Workers::Failed(error) = &self.workers {
            return Some(Arc::new(Error::Worker(error.clone())));
        }
        self.output
            .lock()
            .ok()
            .and_then(|out| out.audio_error.clone().or_else(|| out.error.clone()))
    }
    pub fn frame(&self) -> Option<&Arc<Frame>> {
        self.shown.as_ref()
    }
    /// Advance presentation and device health using the caller's monotonic time.
    /// Only callback progress refreshes health; this clock never drives transport.
    pub fn poll(&mut self, now: std::time::Instant) -> Result<()> {
        self.poll_workers()?;
        if matches!(self.workers, Workers::Restarting) {
            return Ok(());
        }
        let submitted = self.port.submitted.load(Ordering::Acquire);
        if submitted != self.last_callback.0 {
            self.last_callback = (submitted, now);
        }
        if self.playing()
            && (self.port.failed()
                || now.saturating_duration_since(self.last_callback.1) > Duration::from_secs(1))
        {
            let r = self.request.lock().map_err(|_| Error::Poisoned)?.clone();
            self.seek(self.position()?, false, r.project)?;
            return Err(Error::PreviewUnavailable("audio clock stalled"));
        }
        let request = self.request.lock().map_err(|_| Error::Poisoned)?.clone();
        let at = request.display_at(self.port.audible.load(Ordering::Acquire))?;
        {
            let mut output = self.output.lock().map_err(|_| Error::Poisoned)?;
            while output
                .frames
                .front()
                .is_some_and(|f| f.generation != request.generation || f.at <= at)
            {
                if let Some(frame) = output.frames.pop_front()
                    && frame.generation == request.generation
                {
                    self.shown = Some(frame);
                }
            }
        }
        let transport_at = request.at(self.port.audible.load(Ordering::Acquire))?;
        if request.playing && transport_at >= request.project.duration() {
            self.seek(transport_at, false, request.project)?;
            // Automatic end stops transport; a pointer still over the timeline
            // keeps its preview until leave, OFF or an explicit transport action.
            self.set_hover(request.hover)?;
        }
        Ok(())
    }
}
impl Drop for Playback {
    fn drop(&mut self) {
        for cancel in &self.cancels {
            cancel.cancel();
        }
        self.runner.shutdown();
    }
}
/// Map `at` into a looping `[from, to)` range; one rule for ticks and samples.
/// Positions before `to` pass through, and an empty range never wraps.
fn wrap_units(at: i64, from: i64, to: i64) -> i64 {
    if at < to || to <= from {
        at
    } else {
        from + (at - from).rem_euclid(to - from)
    }
}

/// The loop range the transport holds. `in_out` is the export in/out marker pair; an
/// unset, inverted or empty pair falls back to the whole timeline. Disabled or an empty
/// timeline gives `None`.
pub fn loop_window(
    enabled: bool,
    in_out: Option<(Tick, Tick)>,
    duration: Tick,
) -> Option<(Tick, Tick)> {
    if !enabled {
        return None;
    }
    if let Some((start, end)) = in_out {
        let start = start.clamp(Tick::ZERO, duration);
        let end = end.clamp(Tick::ZERO, duration);
        if start < end {
            return Some((start, end));
        }
    }
    (duration > Tick::ZERO).then_some((Tick::ZERO, duration))
}

/// One output block of `frames` samples: `head` samples from `start`, then, when the loop
/// wraps inside the block, `rest` = (loop start, frames) continues it.
#[derive(Debug, PartialEq, Eq)]
struct BlockPlan {
    start: i64,
    head: usize,
    rest: Option<(i64, usize)>,
}

fn plan_block(first: i64, bounds: Option<(i64, i64)>, frames: usize) -> BlockPlan {
    let Some((from, to)) = bounds else {
        return BlockPlan {
            start: first,
            head: frames,
            rest: None,
        };
    };
    let start = wrap_units(first, from, to);
    let head = (to - start).clamp(1, frames as i64) as usize;
    BlockPlan {
        start,
        head,
        rest: (head < frames).then_some((from, frames - head)),
    }
}

#[cfg(test)]
impl BlockPlan {
    /// The timeline sample each output slot reads, in order.
    fn indices(&self) -> Vec<i64> {
        let mut out: Vec<i64> = (0..self.head as i64).map(|n| self.start + n).collect();
        if let Some((from, frames)) = self.rest {
            out.extend((0..frames as i64).map(|n| from + n));
        }
        out
    }
}
fn read_request(request: &Mutex<Request>) -> Result<Request> {
    Ok(request.lock().map_err(|_| Error::Poisoned)?.clone())
}
/// Cancel a long seek's deterministic preroll when a newer request supersedes it.
struct AudioEpoch<'a> {
    context: &'a engine::jobs::JobContext,
    port: &'a AudioPort,
    epoch: u64,
}
impl render::ExecutionHooks for AudioEpoch<'_> {
    fn cancelled(&self) -> bool {
        self.context.cancelled() || self.port.epoch.load(Ordering::Acquire) != self.epoch
    }
    fn reserve(
        &self,
        kind: render::MemoryKind,
        bytes: u64,
    ) -> render::Result<Box<dyn render::Reservation>> {
        render::ExecutionHooks::reserve(self.context, kind, bytes)
    }
}
fn audio_loop(
    request: &Mutex<Request>,
    port: &AudioPort,
    factory: &dyn Sources,
    hooks: &engine::jobs::JobContext,
) -> Result<()> {
    let r = read_request(request)?;
    if !r.playing {
        std::thread::sleep(Duration::from_millis(2));
        return Ok(());
    }
    let mut sources = factory.create();
    let epoch = r.epoch;
    let mut cursor = port.submitted.load(Ordering::Acquire);
    let audio_hooks = AudioEpoch {
        context: hooks,
        port,
        epoch,
    };
    // DSP state belongs only to a playing epoch, and drops on pause/seek.
    let rate = i64::from(r.project.data().settings.audio_sample_rate);
    // Loop bounds in samples. The first block takes its position from the transport;
    // later blocks continue sample by sample, so the loop boundary has no gap.
    let bounds = match r.loop_bounds() {
        Some((from, to)) => Some((
            from.to_units(rate, Rounding::Nearest)?,
            to.to_units(rate, Rounding::Nearest)?,
        )),
        None => None,
    };
    let mut carried: Option<i64> = None;
    let mut mixer = render::AudioMixer::new(&r.project, 2, 0, &audio_hooks)?;
    while !hooks.cancelled() && port.epoch.load(Ordering::Acquire) == epoch {
        let submitted = port.submitted.load(Ordering::Acquire);
        cursor = cursor.max(submitted);
        let lead = u64::from(r.project.data().settings.audio_sample_rate) / 4;
        if cursor > submitted.saturating_add(lead) {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        let first = match carried {
            Some(sample) => sample,
            None => r.at(cursor)?.to_units(rate, Rounding::Nearest)?,
        };
        // A looping block stops at the loop end; its remainder comes from the loop start.
        let plan = plan_block(first, bounds, BLOCK);
        let head = plan.head;
        let block = mixer.mix(plan.start, head, sources.as_mut(), &audio_hooks)?;
        let rest = match plan.rest {
            Some((from, frames)) => {
                Some(mixer.mix(from, frames, sources.as_mut(), &audio_hooks)?)
            }
            None => None,
        };
        if port.epoch.load(Ordering::Acquire) == epoch {
            for n in 0..BLOCK {
                let (source, k) = match rest.as_ref() {
                    Some(rest) if n >= head => (rest, n - head),
                    _ => (&block, n),
                };
                let index = cursor.saturating_add(n as u64);
                if let Some(slot) = port.slots.get(index as usize % RING_FRAMES) {
                    slot.epoch.store(0, Ordering::Release);
                    slot.left.store(
                        source
                            .planes()
                            .first()
                            .and_then(|p| p.get(k))
                            .copied()
                            .unwrap_or(0.0)
                            .to_bits(),
                        Ordering::Relaxed,
                    );
                    slot.right.store(
                        source
                            .planes()
                            .get(1)
                            .and_then(|p| p.get(k))
                            .copied()
                            .unwrap_or(0.0)
                            .to_bits(),
                        Ordering::Relaxed,
                    );
                    slot.index.store(index, Ordering::Relaxed);
                    slot.epoch.store(epoch, Ordering::Release);
                }
            }
        }
        cursor = cursor.saturating_add(BLOCK as u64);
        carried = bounds.map(|_| plan.start.saturating_add(BLOCK as i64));
    }
    Ok(())
}
fn video_loop(
    request: &Mutex<Request>,
    output: &Mutex<Output>,
    port: &AudioPort,
    generation: &AtomicU64,
    budget: &ByteBudget,
    factory: &dyn Sources,
    hooks: &engine::jobs::JobContext,
) -> Result<()> {
    let r = read_request(request)?;
    let preview_generation = r.generation;
    let mut sources = factory.create();
    let fps = r.project.data().canvas.fps;
    // A held pointer time is one frame: never decode ahead of it, and never
    // advance it with the audio clock.
    let pinned = !r.playing || r.hover.is_some();
    let mut next = fps.frame_at(r.display_at(port.audible.load(Ordering::Acquire))?)?;
    let mut last = next;
    let looping = r.loop_bounds().is_some();
    let mut settings = r.render_state.settings(&r.project)?;
    let scale = (640.0 / f64::from(settings.width))
        .min(360.0 / f64::from(settings.height))
        .min(1.0);
    settings.width = (f64::from(settings.width) * scale).round().max(1.0) as u32;
    settings.height = (f64::from(settings.height) * scale).round().max(1.0) as u32;
    settings.effects = r.effects.clone();
    let mut renderer = render::Renderer::new(&r.project, settings)?;
    while !hooks.cancelled() && generation.load(Ordering::Acquire) == preview_generation {
        let current = fps.frame_at(r.display_at(port.audible.load(Ordering::Acquire))?)?;
        if looping && current < last {
            // The transport wrapped: frames rendered past the loop end are stale.
            next = current;
            output.lock().map_err(|_| Error::Poisoned)?.frames.clear();
        }
        last = current;
        next = next.max(current);
        if (pinned && next > current)
            || output.lock().map_err(|_| Error::Poisoned)?.frames.len() >= MAX_FRAMES
        {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        let at = fps.tick_of(next)?;
        let image = renderer.render_frame(at, sources.as_mut(), hooks)?;
        let lease = budget.reserve(
            ByteOwner::DecodedFrames,
            u64::from(image.width()) * u64::from(image.height()) * 12,
        )?;
        // The player paints over black; composite there in linear light before
        // egui applies its transfer-encoded alpha blending.
        let rgba = image.video_rgba8(13, hooks)?;
        let frame = Arc::new(Frame {
            generation: preview_generation,
            at,
            image: egui::ColorImage::from_rgba_unmultiplied(
                [image.width() as usize, image.height() as usize],
                &rgba.bytes,
            ),
            _lease: lease,
        });
        let mut out = output.lock().map_err(|_| Error::Poisoned)?;
        if generation.load(Ordering::Acquire) == preview_generation {
            out.frames.push_back(frame);
        }
        next = next
            .checked_add(1)
            .ok_or(Error::Invalid("frame overflow"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::{Files, Playback, loop_window, plan_block, wrap_units};
    use engine::{ByteBudget, ByteOwner, Session};
    use std::sync::Arc;
    use time::Tick;

    #[test]
    fn a_successor_opens_on_a_full_budget_and_shares_the_ring_bytes() {
        let session = Session::new(
            project::Project::empty(project::ProjectId(1), "full".into()).unwrap(),
            10,
        )
        .unwrap();
        let budget = ByteBudget::new(64 << 20).unwrap();
        let playback = Playback::new(&session, budget.clone(), Arc::new(Files)).unwrap();
        let ring = (super::RING_FRAMES * std::mem::size_of::<super::Slot>()) as u64;
        let snapshot = budget.snapshot().unwrap();
        let pressure = budget
            .reserve(ByteOwner::Export, snapshot.limit - snapshot.used)
            .unwrap();
        assert!(Playback::new(&session, budget.clone(), Arc::new(Files)).is_err());
        let successor = playback.successor(&session, Arc::new(Files)).unwrap();
        assert!(!Arc::ptr_eq(
            &playback.audio_port(),
            &successor.audio_port()
        ));
        drop(pressure);
        drop(playback);
        assert!(
            budget.snapshot().unwrap().used >= ring,
            "the successor keeps the ring"
        );
        drop(successor);
    }

    type Bounds = Option<(i64, i64)>;

    #[test]
    fn loop_wrap_rows() {
        // (position, loop start, loop end, expected)
        let rows = [
            (0, 0, 10, 0),
            (9, 0, 10, 9),
            (10, 0, 10, 0),
            (25, 0, 10, 5),
            (12, 4, 10, 6),
            (3, 4, 10, 3),
            // An in/out range wraps at its out point back to its in point.
            (10, 4, 10, 4),
            (13, 4, 10, 7),
            // An empty range never wraps.
            (7, 5, 5, 7),
        ];
        for (at, from, to, expected) in rows {
            assert_eq!(
                wrap_units(at, from, to),
                expected,
                "at {at} in {from}..{to}"
            );
        }
    }

    #[test]
    fn loop_window_rows() {
        // (enabled, in/out ticks, duration, expected window)
        let rows: [(bool, Bounds, i64, Bounds); 9] = [
            (false, Some((10, 50)), 100, None),
            (true, None, 100, Some((0, 100))),
            (true, Some((10, 50)), 100, Some((10, 50))),
            (true, Some((-5, 30)), 100, Some((0, 30))),
            (true, Some((40, 200)), 100, Some((40, 100))),
            (true, Some((50, 50)), 100, Some((0, 100))),
            (true, Some((60, 20)), 100, Some((0, 100))),
            (true, None, 0, None),
            (true, Some((0, 0)), 0, None),
        ];
        for (enabled, in_out, duration, expected) in rows {
            let in_out = in_out.map(|(a, b)| (Tick(a), Tick(b)));
            let expected = expected.map(|(a, b)| (Tick(a), Tick(b)));
            assert_eq!(
                loop_window(enabled, in_out, Tick(duration)),
                expected,
                "enabled {enabled} in/out {in_out:?} duration {duration}"
            );
        }
    }

    #[test]
    fn loop_block_split_rows() {
        // (first sample, loop bounds, block frames, timeline sample per output slot)
        let rows: [(i64, Bounds, usize, Vec<i64>); 6] = [
            (2, None, 4, vec![2, 3, 4, 5]),
            (2, Some((0, 10)), 4, vec![2, 3, 4, 5]),
            // The block crosses the loop end: the slots after 9 read the loop start, 0 and 1.
            (8, Some((0, 10)), 4, vec![8, 9, 0, 1]),
            (10, Some((0, 10)), 4, vec![0, 1, 2, 3]),
            (9, Some((4, 10)), 4, vec![9, 4, 5, 6]),
            (25, Some((0, 10)), 4, vec![5, 6, 7, 8]),
        ];
        for (first, bounds, frames, expected) in rows {
            let plan = plan_block(first, bounds, frames);
            assert_eq!(plan.indices(), expected, "first {first} bounds {bounds:?}");
            // Continuity: the slot after the loop end reads the loop start sample.
            if let Some((from, _)) = plan.rest {
                assert_eq!(expected.get(plan.head).copied(), Some(from));
            }
        }
    }
}
