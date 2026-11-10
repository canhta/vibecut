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

/// Render's source seam is injected; production uses the same FileSources as export.
pub trait Sources: Send + Sync + 'static {
    fn create(&self) -> Box<dyn render::SourceProvider>;
}
pub struct Files;
impl Sources for Files {
    fn create(&self) -> Box<dyn render::SourceProvider> {
        Box::new(render::FileSources::default())
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
    _lease: ByteLease,
}
impl AudioPort {
    fn new(budget: &ByteBudget) -> Result<Self> {
        let lease = budget.reserve(
            ByteOwner::DecodedFrames,
            (RING_FRAMES * std::mem::size_of::<Slot>()) as u64,
        )?;
        Ok(Self {
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
            _lease: lease,
        })
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
}
#[derive(Clone)]
struct Request {
    render_state: Arc<RenderState>,
    effects: Option<Arc<dyn render::effects::EffectCompositor>>,
    epoch: u64,
    project: Arc<project::Project>,
    start: Tick,
    origin: u64,
    playing: bool,
    /// Transient preview-axis time; `None` follows the transport.
    hover: Option<Tick>,
}
impl Request {
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
        Ok(self
            .start
            .checked_add(Tick::from_units(
                delta,
                i64::from(self.project.data().settings.audio_sample_rate),
                Rounding::Nearest,
            )?)
            .ok_or(Error::Invalid("transport overflow"))?
            .min(self.project.duration()))
    }
}
pub struct Frame {
    pub epoch: u64,
    pub at: Tick,
    pub image: egui::ColorImage,
    /// Covers CPU ColorImage, egui upload and GPU texture retention. Native driver
    /// staging/allocator overhead is measured separately, never claimed as capped RSS.
    _lease: ByteLease,
}
struct Output {
    frames: VecDeque<Arc<Frame>>,
    error: Option<Arc<Error>>,
}
enum Workers {
    Running,
    Failed(Arc<engine::EditError>),
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
    pub fn new(session: &Session, budget: ByteBudget, sources: Arc<dyn Sources>) -> Result<Self> {
        let port = Arc::new(AudioPort::new(&budget)?);
        let request = Arc::new(Mutex::new(Request {
            render_state: Arc::new(RenderState::default()),
            effects: None,
            epoch: 1,
            project: session.project().clone(),
            start: Tick::ZERO,
            origin: 0,
            playing: false,
            hover: None,
        }));
        let output = Arc::new(Mutex::new(Output {
            frames: VecDeque::new(),
            error: None,
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
            let (request, output, port, sources, budget) = (
                self.request.clone(),
                self.output.clone(),
                self.port.clone(),
                self.sources.clone(),
                self.budget.clone(),
            );
            let submitted = self.runner.submit(self.spec, move |hooks| {
                while !hooks.cancelled() {
                    let epoch = port.epoch.load(Ordering::Acquire);
                    let result = if audio {
                        audio_loop(&request, &port, sources.as_ref(), hooks)
                    } else {
                        video_loop(&request, &output, &port, &budget, sources.as_ref(), hooks)
                    };
                    if let Err(error) = result
                        && !hooks.cancelled()
                    {
                        if port.epoch.load(Ordering::Acquire) == epoch
                            && let Ok(mut out) = output.lock()
                        {
                            out.error = Some(Arc::new(error));
                        }
                        while !hooks.cancelled() && port.epoch.load(Ordering::Acquire) == epoch {
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
    pub fn budget(&self) -> &ByteBudget {
        &self.budget
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
    /// Snapshot generation, bumped by every seek. Audio ring samples and
    /// preview frames carry it, so callers can tell restored-snapshot output
    /// from work produced for a superseded project.
    pub fn generation(&self) -> u64 {
        self.port.epoch.load(Ordering::Acquire)
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
        self.port.playing.store(false, Ordering::Release);
        let epoch = request
            .epoch
            .checked_add(1)
            .ok_or(Error::Invalid("preview generation exhausted"))?;
        *request = Request {
            render_state: request.render_state.clone(),
            effects: request.effects.clone(),
            epoch,
            project,
            start: at,
            origin: self.port.submitted.load(Ordering::Acquire),
            playing,
            hover: None,
        };
        self.port.epoch.store(epoch, Ordering::Release);
        self.port.playing.store(playing, Ordering::Release);
        drop(request);
        self.shown = None;
        let mut output = self.output.lock().map_err(|_| Error::Poisoned)?;
        output.frames.clear();
        output.error = None;
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
        let at = at.map(|at| at.clamp(Tick::ZERO, request.project.duration()));
        if request.hover == at {
            return Ok(());
        }
        let epoch = request
            .epoch
            .checked_add(1)
            .ok_or(Error::Invalid("preview generation exhausted"))?;
        request.hover = at;
        request.epoch = epoch;
        self.port.epoch.store(epoch, Ordering::Release);
        drop(request);
        self.shown = None;
        let mut output = self.output.lock().map_err(|_| Error::Poisoned)?;
        output.frames.clear();
        output.error = None;
        self.last_callback = (
            self.port.submitted.load(Ordering::Acquire),
            std::time::Instant::now(),
        );
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
        self.output.lock().ok().and_then(|out| out.error.clone())
    }
    pub fn frame(&self) -> Option<&Arc<Frame>> {
        self.shown.as_ref()
    }
    pub fn poll(&mut self) -> Result<()> {
        self.poll_workers()?;
        if matches!(self.workers, Workers::Restarting) {
            return Ok(());
        }
        let submitted = self.port.submitted.load(Ordering::Acquire);
        if submitted != self.last_callback.0 {
            self.last_callback = (submitted, std::time::Instant::now());
        }
        if self.playing()
            && (self.port.failed() || self.last_callback.1.elapsed() > Duration::from_secs(1))
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
                .is_some_and(|f| f.epoch != request.epoch || f.at <= at)
            {
                if let Some(frame) = output.frames.pop_front()
                    && frame.epoch == request.epoch
                {
                    self.shown = Some(frame);
                }
            }
        }
        if request.playing && request.hover.is_none() && at >= request.project.duration() {
            self.seek(at, false, request.project)?;
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
    let mut mixer = render::AudioMixer::new(&r.project, 2, 0, &audio_hooks)?;
    while !hooks.cancelled() && port.epoch.load(Ordering::Acquire) == epoch {
        let submitted = port.submitted.load(Ordering::Acquire);
        cursor = cursor.max(submitted);
        let lead = u64::from(r.project.data().settings.audio_sample_rate) / 4;
        if cursor > submitted.saturating_add(lead) {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        let at = r.at(cursor)?;
        let start = at.to_units(
            i64::from(r.project.data().settings.audio_sample_rate),
            Rounding::Nearest,
        )?;
        let block = mixer.mix(start, BLOCK, sources.as_mut(), &audio_hooks)?;
        if port.epoch.load(Ordering::Acquire) == epoch {
            for n in 0..BLOCK {
                let index = cursor.saturating_add(n as u64);
                if let Some(slot) = port.slots.get(index as usize % RING_FRAMES) {
                    slot.epoch.store(0, Ordering::Release);
                    slot.left.store(
                        block
                            .planes()
                            .first()
                            .and_then(|p| p.get(n))
                            .copied()
                            .unwrap_or(0.0)
                            .to_bits(),
                        Ordering::Relaxed,
                    );
                    slot.right.store(
                        block
                            .planes()
                            .get(1)
                            .and_then(|p| p.get(n))
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
    }
    Ok(())
}
fn video_loop(
    request: &Mutex<Request>,
    output: &Mutex<Output>,
    port: &AudioPort,
    budget: &ByteBudget,
    factory: &dyn Sources,
    hooks: &engine::jobs::JobContext,
) -> Result<()> {
    while !hooks.cancelled() {
        let r = read_request(request)?;
        let epoch = r.epoch;
        let mut sources = factory.create();
        let fps = r.project.data().canvas.fps;
        // A held pointer time is one frame: never decode ahead of it, and never
        // advance it with the audio clock.
        let pinned = !r.playing || r.hover.is_some();
        let mut next = fps.frame_at(r.display_at(port.audible.load(Ordering::Acquire))?)?;
        let mut settings = r.render_state.settings(&r.project)?;
        let scale = (640.0 / f64::from(settings.width))
            .min(360.0 / f64::from(settings.height))
            .min(1.0);
        settings.width = (f64::from(settings.width) * scale).round().max(1.0) as u32;
        settings.height = (f64::from(settings.height) * scale).round().max(1.0) as u32;
        settings.effects = r.effects.clone();
        let mut renderer = render::Renderer::new(&r.project, settings)?;
        while !hooks.cancelled() && port.epoch.load(Ordering::Acquire) == epoch {
            let current = fps.frame_at(r.display_at(port.audible.load(Ordering::Acquire))?)?;
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
                epoch,
                at,
                image: egui::ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    &rgba.bytes,
                ),
                _lease: lease,
            });
            let mut out = output.lock().map_err(|_| Error::Poisoned)?;
            if port.epoch.load(Ordering::Acquire) == epoch {
                out.frames.push_back(frame);
            }
            next = next
                .checked_add(1)
                .ok_or(Error::Invalid("frame overflow"))?;
        }
    }
    Ok(())
}
