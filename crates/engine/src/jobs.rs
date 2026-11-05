//! Caller-driven bounded scheduling of blocking domain execution.
use crate::cache::CacheKey;
use crate::{ByteBudget, ByteLease, ByteOwner, Command, EditError, JobTarget, Outcome, Session};
use std::{
    collections::{BTreeMap, VecDeque},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub struct JobId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceClass {
    Cpu,
    GpuEncoder,
    Network,
}
impl ResourceClass {
    fn index(self) -> usize {
        match self {
            Self::Cpu => 0,
            Self::GpuEncoder => 1,
            Self::Network => 2,
        }
    }
}
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Transcribe,
    Translate,
    SuggestClips,
    Preview,
    Export,
    Import,
    Persist,
    DownloadModel,
}
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum JobStage {
    Queued,
    Starting,
    Transcribe,
    Align,
    Translate,
    SuggestClips,
    Assemble,
    Process,
    Admit,
    Render,
    Mix,
    Encode,
    Finalize,
    Download,
    Verify,
    Publish,
    Backoff,
    Complete,
}
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Cancelling,
    Publishing,
    Backoff,
    Completed,
    Cancelled,
    Failed,
}
/// Caller supplies a monotonic clock; tests advance it without sleeping for backoff.
pub trait Clock: Send + Sync {
    fn now(&self) -> Duration;
}
pub struct SystemClock(Instant);
impl Default for SystemClock {
    fn default() -> Self {
        Self(Instant::now())
    }
}
impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}
#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicU8>);
impl CancelToken {
    pub fn cancel(&self) {
        let _ = self
            .0
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
    }
    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire) == 1
    }
}
#[derive(Clone)]
pub struct JobSnapshot {
    pub id: JobId,
    pub kind: JobKind,
    pub input: CacheKey,
    pub stage: JobStage,
    /// Finite normalized progress, 0..=1. Failed/cancelled jobs never imply completion.
    pub progress: f32,
    pub state: JobState,
    pub attempts: u8,
    pub error: Option<Arc<EditError>>,
}
/// Reservations bound actual adapter allocations, not just worker counts. The
/// adapter must include both overlapping batches and all temporary storage here.
#[derive(Clone, Copy, Default)]
pub struct JobBytes {
    pub decoded_frames: u64,
    pub compositor: u64,
    pub export: u64,
    pub result: u64,
}
#[derive(Clone, Copy)]
pub struct JobSpec {
    pub kind: JobKind,
    pub class: ResourceClass,
    pub input: CacheKey,
    pub bytes: JobBytes,
}
/// Network retries apply only to Transcribe/Translate + NetworkTransient; local
/// execution and Export are never retried. Every retry repeats consent validation
/// in the adapter and releases worker/resources during backoff.
#[derive(Clone, Copy)]
pub struct RunnerConfig {
    pub concurrency: [usize; 3],
    pub queue: usize,
    pub retained: usize,
    pub network_attempts: u8,
    pub backoff: Duration,
    pub progress_interval: Duration,
}
struct Progress {
    stage: JobStage,
    value: f32,
    at: Duration,
}
#[derive(Clone)]
pub struct JobContext {
    budget: ByteBudget,
    cancel: CancelToken,
    progress: Arc<Mutex<Progress>>,
    clock: Arc<dyn Clock>,
    interval: Duration,
}
impl JobContext {
    pub(crate) fn memory_budget(&self) -> &ByteBudget {
        &self.budget
    }
    /// Whether an irreversible publication has closed the cancellation window.
    pub fn publication_started(&self) -> bool {
        self.cancel.0.load(Ordering::Acquire) == 2
    }
    /// Atomically close cancellation before one irreversible output publication.
    /// Late cancel requests are ignored; failed publication still returns its error.
    pub fn begin_publication(&self) -> Result<(), EditError> {
        match self
            .cancel
            .0
            .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) | Err(2) => Ok(()),
            _ => Err(EditError::Cancelled),
        }
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.cancelled()
    }
    pub fn check_cancelled(&self) -> Result<(), EditError> {
        if self.cancelled() {
            Err(EditError::Cancelled)
        } else {
            Ok(())
        }
    }
    /// Seal a verified atomic publication against cancellation. Cancellation that
    /// wins first refuses publication; after sealing, cancel is a no-op and the
    /// operation finishes or fails with its IO result. No mutex spans IO.
    pub fn commit_publication<T>(
        &self,
        publish: impl FnOnce() -> Result<T, EditError>,
    ) -> Result<T, EditError> {
        self.begin_publication()?;
        {
            let mut progress = self.progress.lock().map_err(|_| EditError::JobPoisoned)?;
            progress.stage = JobStage::Publish;
            progress.at = self.clock.now();
        }
        publish()
    }
    /// Latest value only, at most once per configured interval (including stage
    /// changes). Invalid values are refused without changing visible progress.
    pub fn progress(&self, stage: JobStage, value: f32) -> Result<(), EditError> {
        self.check_cancelled()?;
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(EditError::Invalid("job progress"));
        }
        let now = self.clock.now();
        let mut p = self.progress.lock().map_err(|_| EditError::JobPoisoned)?;
        if now.saturating_sub(p.at) >= self.interval {
            *p = Progress {
                stage,
                value,
                at: now,
            };
        }
        Ok(())
    }
}
impl ai::ExecutionHooks for JobContext {
    fn cancelled(&self) -> bool {
        self.cancelled()
    }
    fn stage(&self, stage: ai::Stage) {
        self.progress(stage.into(), 0.0).ok();
    }
    fn progress(&self, stage: ai::Stage, percent: u8) {
        self.progress(stage.into(), f32::from(percent.min(100)) / 100.0)
            .ok();
    }
    fn cancelling(&self) {
        self.cancel.cancel();
    }
}
impl From<ai::Stage> for JobStage {
    fn from(stage: ai::Stage) -> Self {
        match stage {
            ai::Stage::Transcribe => Self::Transcribe,
            ai::Stage::Align => Self::Align,
            ai::Stage::Translate => Self::Translate,
            ai::Stage::SuggestClips => Self::SuggestClips,
            ai::Stage::Vision => Self::Process,
            ai::Stage::Assemble => Self::Assemble,
        }
    }
}
type Task = Arc<dyn Fn(&JobContext) -> Result<Vec<u8>, EditError> + Send + Sync>;
struct Pending {
    target: Option<JobTarget>,
    spec: JobSpec,
    snapshot: JobSnapshot,
    task: Task,
    context: JobContext,
    ready: Duration,
}
struct Running {
    pending: Pending,
    handle: JoinHandle<Result<Vec<u8>, EditError>>,
    _work: Vec<ByteLease>,
    result: ByteLease,
}
/// Output is promoted only after the task returned successfully, cancellation was
/// checked and its maximum declared byte size was validated. It is never applied
/// to a Session implicitly; use Session::apply_job_result with its captured target.
pub struct JobOutput {
    target: Option<JobTarget>,
    pub id: JobId,
    pub input: CacheKey,
    bytes: Vec<u8>,
    _charge: ByteLease,
}
impl JobOutput {
    /// Apply with the Session identity captured by submit_for_session, never a
    /// freshly supplied revision. Non-session artifact jobs cannot apply edits.
    pub fn apply(&self, session: &mut Session, command: Command) -> Result<Outcome, EditError> {
        let target = self
            .target
            .as_ref()
            .ok_or(EditError::Unsupported("job has no Session target"))?;
        session.apply_job_result(target, command)
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
struct Finished {
    snapshot: JobSnapshot,
    output: Option<JobOutput>,
}
pub struct JobRunner {
    config: RunnerConfig,
    budget: ByteBudget,
    clock: Arc<dyn Clock>,
    next: u64,
    queue: VecDeque<Pending>,
    running: Vec<Running>,
    finished: VecDeque<Finished>,
    events: BTreeMap<JobId, JobSnapshot>,
}
impl JobRunner {
    pub fn new(
        config: RunnerConfig,
        budget: ByteBudget,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, EditError> {
        if config.concurrency.iter().any(|n| *n == 0 || *n > 16)
            || !(1..=10_000).contains(&config.queue)
            || !(1..=10_000).contains(&config.retained)
            || !(1..=10).contains(&config.network_attempts)
            || config.backoff.is_zero()
            || config.progress_interval < Duration::from_millis(10)
        {
            return Err(EditError::Limit);
        }
        Ok(Self {
            config,
            budget,
            clock,
            next: 0,
            queue: VecDeque::new(),
            running: Vec::new(),
            finished: VecDeque::new(),
            events: BTreeMap::new(),
        })
    }
    /// Typed nonblocking queue-full refusal. Closures must have bounded captures,
    /// obey reserved bytes and cancellation. Persist jobs may publish complete atomic
    /// journal/project files; other jobs must not publish partial files.
    /// Returning bytes cannot promote partial work. Encoded export restart is unsupported.
    pub fn submit(
        &mut self,
        spec: JobSpec,
        task: impl Fn(&JobContext) -> Result<Vec<u8>, EditError> + Send + Sync + 'static,
    ) -> Result<(JobId, CancelToken), EditError> {
        self.submit_target(spec, None, task)
    }
    /// Bind returned output to exactly this Session revision before scheduling.
    pub fn submit_for_session(
        &mut self,
        session: &Session,
        spec: JobSpec,
        task: impl Fn(&JobContext) -> Result<Vec<u8>, EditError> + Send + Sync + 'static,
    ) -> Result<(JobId, CancelToken), EditError> {
        self.submit_target(spec, Some(session.job_target()), task)
    }
    fn submit_target(
        &mut self,
        spec: JobSpec,
        target: Option<JobTarget>,
        task: impl Fn(&JobContext) -> Result<Vec<u8>, EditError> + Send + Sync + 'static,
    ) -> Result<(JobId, CancelToken), EditError> {
        if self.queue.len() >= self.config.queue {
            return Err(EditError::QueueFull);
        }
        let total = [
            spec.bytes.decoded_frames,
            spec.bytes.compositor,
            spec.bytes.export,
            spec.bytes.result,
        ]
        .into_iter()
        .try_fold(0u64, |a, b| a.checked_add(b))
        .ok_or(EditError::ArtifactTooLarge)?;
        if total > self.budget.snapshot()?.limit {
            return Err(EditError::ArtifactTooLarge);
        }
        let id = JobId(self.next);
        self.next = self.next.checked_add(1).ok_or(EditError::IdExhausted)?;
        let context = JobContext {
            budget: self.budget.clone(),
            cancel: CancelToken::default(),
            progress: Arc::new(Mutex::new(Progress {
                stage: JobStage::Starting,
                value: 0.0,
                at: self.clock.now(),
            })),
            clock: self.clock.clone(),
            interval: self.config.progress_interval,
        };
        let snapshot = JobSnapshot {
            id,
            kind: spec.kind,
            input: spec.input,
            stage: JobStage::Queued,
            progress: 0.0,
            state: JobState::Queued,
            attempts: 0,
            error: None,
        };
        self.events.insert(id, snapshot.clone());
        let token = context.cancel.clone();
        self.queue.push_back(Pending {
            target,
            spec,
            snapshot,
            task: Arc::new(task),
            context,
            ready: self.clock.now(),
        });
        Ok((id, token))
    }
    /// Bound a batch by both resource concurrency and retained terminal results.
    pub(crate) fn batch_capacity(&self, class: ResourceClass) -> usize {
        self.config
            .concurrency
            .get(class.index())
            .copied()
            .unwrap_or(1)
            .min(self.config.retained)
    }

    pub(crate) fn uses_budget(&self, budget: &ByteBudget) -> bool {
        self.budget.same_policy(budget)
    }
    /// Pump from the UI/CLI event loop. No wait for executing tasks; completed
    /// handles are joined. Scheduling scans past resource-blocked jobs.
    pub fn tick(&mut self) -> Result<(), EditError> {
        let now = self.clock.now();
        let mut i = 0;
        while i < self.running.len() {
            if !self.running.get(i).is_some_and(|r| r.handle.is_finished()) {
                i += 1;
                continue;
            }
            let running = self.running.remove(i);
            let Running {
                mut pending,
                handle,
                _work,
                result: charge,
            } = running;
            let outcome = handle.join().unwrap_or(Err(EditError::WorkerPanicked));
            drop(_work);
            let outcome = if pending.context.cancelled() {
                Err(EditError::Cancelled)
            } else {
                outcome
            };
            let transient = match &outcome {
                Err(EditError::NetworkTransient) => true,
                Err(EditError::Ai(error)) => error.is_transient(),
                _ => false,
            };
            let retry = transient
                && !pending.context.publication_started()
                && pending.spec.class == ResourceClass::Network
                && matches!(
                    pending.spec.kind,
                    JobKind::Transcribe | JobKind::Translate | JobKind::DownloadModel
                )
                && pending.snapshot.attempts < self.config.network_attempts;
            // A retry must occupy one bounded pending slot. If that slot filled
            // while running, retain the typed failure instead of growing the queue.
            if retry && self.queue.len() < self.config.queue {
                let backoff = self
                    .config
                    .backoff
                    .saturating_mul(1u32 << (pending.snapshot.attempts - 1));
                // Provider delays are advisory and never allowed to park a job for
                // an unbounded time; the runner's attempt budget still applies.
                let retry_after = match &outcome {
                    Err(EditError::Ai(ai::Error::RateLimit {
                        retry_after_seconds: Some(seconds),
                    })) => Some(Duration::from_secs(u64::from(*seconds).min(300))),
                    _ => None,
                };
                pending.ready =
                    now.saturating_add(retry_after.map_or(backoff, |at| backoff.max(at)));
                pending.snapshot.state = JobState::Backoff;
                pending.snapshot.stage = JobStage::Backoff;
                self.events
                    .insert(pending.snapshot.id, pending.snapshot.clone());
                self.queue.push_back(pending);
                drop(charge);
                continue;
            }
            self.finish(pending, outcome, Some(charge));
        }
        for running in &mut self.running {
            let p = running
                .pending
                .context
                .progress
                .lock()
                .map_err(|_| EditError::JobPoisoned)?;
            let s = &mut running.pending.snapshot;
            if running.pending.context.publication_started() && s.state != JobState::Publishing {
                s.state = JobState::Publishing;
                s.stage = JobStage::Publish;
                s.progress = 1.0;
                self.events.insert(s.id, s.clone());
            }
            if running.pending.context.cancelled() && s.state != JobState::Cancelling {
                s.state = JobState::Cancelling;
                self.events.insert(s.id, s.clone());
            }
            if s.state != JobState::Publishing && (s.stage != p.stage || s.progress != p.value) {
                s.stage = p.stage;
                s.progress = p.value;
                self.events.insert(s.id, s.clone());
            }
        }
        let count = self.queue.len();
        for _ in 0..count {
            let Some(mut pending) = self.queue.pop_front() else {
                break;
            };
            if pending.context.cancelled() {
                self.finish(pending, Err(EditError::Cancelled), None);
                continue;
            }
            if pending.ready > now
                || self
                    .running
                    .iter()
                    .filter(|r| r.pending.spec.class == pending.spec.class)
                    .count()
                    >= self
                        .config
                        .concurrency
                        .get(pending.spec.class.index())
                        .copied()
                        .unwrap_or(0)
            {
                self.queue.push_back(pending);
                continue;
            }
            let mut reservation = self.reserve(pending.spec.bytes);
            while matches!(reservation, Err(EditError::BudgetFull)) && !self.finished.is_empty() {
                if let Some(old) = self.finished.pop_front() {
                    self.events.remove(&old.snapshot.id);
                }
                reservation = self.reserve(pending.spec.bytes);
            }
            let (work, result) = match reservation {
                Ok(v) => v,
                Err(EditError::BudgetFull) => {
                    self.queue.push_back(pending);
                    continue;
                }
                Err(e) => {
                    self.finish(pending, Err(e), None);
                    continue;
                }
            };
            pending.snapshot.attempts += 1;
            pending.snapshot.state = JobState::Running;
            pending.snapshot.stage = JobStage::Starting;
            pending.snapshot.progress = 0.0;
            if let Ok(mut p) = pending.context.progress.lock() {
                *p = Progress {
                    stage: JobStage::Starting,
                    value: 0.0,
                    at: now,
                };
            }
            let context = pending.context.clone();
            let task = pending.task.clone();
            match thread::Builder::new()
                .name("vibecut-job".into())
                .spawn(move || {
                    catch_unwind(AssertUnwindSafe(|| {
                        context.check_cancelled()?;
                        task(&context)
                    }))
                    .unwrap_or(Err(EditError::WorkerPanicked))
                }) {
                Ok(handle) => {
                    self.events
                        .insert(pending.snapshot.id, pending.snapshot.clone());
                    self.running.push(Running {
                        pending,
                        handle,
                        _work: work,
                        result,
                    });
                }
                Err(e) => self.finish(pending, Err(e.into()), None),
            }
        }
        Ok(())
    }
    fn reserve(&self, bytes: JobBytes) -> Result<(Vec<ByteLease>, ByteLease), EditError> {
        let mut work = Vec::new();
        for (owner, bytes) in [
            (ByteOwner::DecodedFrames, bytes.decoded_frames),
            (ByteOwner::Compositor, bytes.compositor),
            (ByteOwner::Export, bytes.export),
        ] {
            work.push(self.budget.reserve(owner, bytes)?);
        }
        Ok((
            work,
            self.budget.reserve(ByteOwner::JobResults, bytes.result)?,
        ))
    }
    fn finish(
        &mut self,
        mut pending: Pending,
        outcome: Result<Vec<u8>, EditError>,
        charge: Option<ByteLease>,
    ) {
        let outcome = outcome.and_then(|bytes| {
            if bytes.capacity() as u64 > pending.spec.bytes.result {
                Err(EditError::ArtifactTooLarge)
            } else {
                Ok(bytes)
            }
        });
        let mut output = None;
        match outcome {
            Ok(bytes) => {
                pending.snapshot.state = JobState::Completed;
                pending.snapshot.stage = JobStage::Complete;
                pending.snapshot.progress = 1.0;
                if let Some(charge) = charge {
                    output = Some(JobOutput {
                        target: pending.target,
                        id: pending.snapshot.id,
                        input: pending.spec.input,
                        bytes,
                        _charge: charge,
                    });
                }
            }
            Err(error) => {
                pending.snapshot.state = if matches!(error, EditError::Cancelled) {
                    JobState::Cancelled
                } else {
                    JobState::Failed
                };
                pending.snapshot.error = Some(Arc::new(error));
            }
        }
        // Drop potentially large captures immediately on completion.
        let snapshot = pending.snapshot;
        self.events.insert(snapshot.id, snapshot.clone());
        self.finished.push_back(Finished { snapshot, output });
        while self.finished.len() > self.config.retained {
            if let Some(old) = self.finished.pop_front() {
                self.events.remove(&old.snapshot.id);
            }
        }
    }
    /// Current retained truth, without draining another consumer's events.
    pub fn snapshots(&self) -> Vec<JobSnapshot> {
        self.queue
            .iter()
            .map(|p| p.snapshot.clone())
            .chain(self.running.iter().map(|r| r.pending.snapshot.clone()))
            .chain(self.finished.iter().map(|f| f.snapshot.clone()))
            .collect()
    }
    /// Cancellation is a request; the existing publication fence can win.
    pub fn cancel_job(&mut self, id: JobId) -> Result<(), EditError> {
        if let Some(p) = self
            .queue
            .iter()
            .chain(self.running.iter().map(|r| &r.pending))
            .find(|p| p.snapshot.id == id)
        {
            p.context.cancel.cancel();
            return Ok(());
        }
        if self.finished.iter().any(|f| f.snapshot.id == id) {
            return Ok(());
        }
        Err(EditError::JobNotRetained)
    }
    pub fn snapshot(&self, id: JobId) -> Option<JobSnapshot> {
        self.queue
            .iter()
            .map(|p| &p.snapshot)
            .chain(self.running.iter().map(|r| &r.pending.snapshot))
            .chain(self.finished.iter().map(|f| &f.snapshot))
            .find(|s| s.id == id)
            .cloned()
    }
    /// Coalescing event channel: latest snapshot per retained/admitted job. Slow
    /// consumers never block workers or accumulate a progress-event history.
    pub fn drain_events(&mut self) -> Vec<JobSnapshot> {
        std::mem::take(&mut self.events).into_values().collect()
    }
    /// Non-consuming view while the existing bounded job retention holds it.
    pub fn output_bytes(&self, id: JobId) -> Option<&[u8]> {
        self.finished
            .iter()
            .find(|f| f.snapshot.id == id)?
            .output
            .as_ref()
            .map(JobOutput::bytes)
    }
    pub fn take_output(&mut self, id: JobId) -> Option<JobOutput> {
        self.finished
            .iter_mut()
            .find(|f| f.snapshot.id == id)?
            .output
            .take()
    }
    /// (pending, active, retained terminal, undrained latest events).
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        (
            self.queue.len(),
            self.running.len(),
            self.finished.len(),
            self.events.len(),
        )
    }
    /// Cancels all work and joins executing adapters. Adapters must cooperate;
    /// blocking IO must have bounded timeouts. Drop follows the same policy.
    pub fn shutdown(&mut self) {
        for p in &self.queue {
            p.context.cancel.cancel();
        }
        for r in &self.running {
            r.pending.context.cancel.cancel();
        }
        self.queue.clear();
        for r in self.running.drain(..) {
            let _ = r.handle.join();
        }
        self.events.clear();
    }
}
impl Drop for JobRunner {
    fn drop(&mut self) {
        self.shutdown();
    }
}
