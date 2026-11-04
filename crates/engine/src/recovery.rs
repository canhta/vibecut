//! Revision-bound session journals. Only the worker serializes project snapshots.
use crate::cache::{ArtifactKind, CacheKey, KeyInputs};
use crate::jobs::{
    Clock, JobBytes, JobId, JobKind, JobRunner, JobSpec, JobState, ResourceClass, RunnerConfig,
};
use crate::{ByteBudget, EditError, JobTarget, Result, Session};
use project::{Project, SaveReceipt, WriterLease};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, TryLockError},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_PROJECT: u64 = 16 * 1024 * 1024;
const MAX_META: u64 = 2 * 1024 * 1024;
const WORK_BYTES: u64 = 40 * 1024 * 1024;
const JOURNAL: &str = "journal.json";

/// Event-loop deadlines; the caller must tick at least as often as its desired latency.
#[derive(Clone, Copy)]
pub struct Schedule {
    pub debounce: Duration,
    pub max_dirty_age: Duration,
}
impl Default for Schedule {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(800),
            max_dirty_age: Duration::from_secs(10),
        }
    }
}
/// Metadata is a journal descriptor, never an alternate project serialization.
/// Debug deliberately omits user names and paths.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    version: u32,
    pub project_id: project::ProjectId,
    pub name: String,
    pub saved_path: Option<PathBuf>,
    pub revision: u64,
    pub content_hash: [u8; 32],
    pub timestamp_unix_ms: u64,
    snapshot: String,
}
/// Holds the abandoned session lock until the caller recovers/discards or drops it.
pub struct Candidate {
    directory: PathBuf,
    _lock: File,
    pub journal: Journal,
    project: Project,
}
impl Candidate {
    /// Open as never-saved work. Its original path remains informational metadata;
    /// save() therefore requires an explicit save-as destination.
    pub fn recover(&self, history_limit: usize) -> Result<Session> {
        Session::new(self.project.clone(), history_limit)
    }
    /// Removes only this journal and its canonical snapshots, preserving the source.
    /// Consumes the claimed candidate; unrelated files are never removed.
    pub fn discard(self) -> Result<()> {
        clear(&self.directory)
    }
}
/// Corrupt candidates have a typed diagnostic; lock/IO failures retain their source.
pub struct Discovery {
    pub candidates: Vec<Candidate>,
    pub diagnostics: Vec<EditError>,
}
fn regular(path: &Path) -> Result<()> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(EditError::RecoveryCorrupt);
    }
    Ok(())
}
fn lock(path: &Path) -> Result<Option<File>> {
    regular(path)?;
    let file = File::options().read(true).write(true).open(path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(lock_error(e)),
    }
}
fn lock_error(e: std::io::Error) -> EditError {
    if e.kind() == std::io::ErrorKind::Unsupported {
        EditError::Unsupported("session locking")
    } else {
        EditError::Io(e)
    }
}
fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>> {
    regular(path)?;
    let mut bytes = Vec::new();
    File::open(path)?.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(EditError::RecoveryCorrupt);
    }
    Ok(bytes)
}
fn candidate(directory: PathBuf, file: File) -> Result<Candidate> {
    let bytes = read_bounded(&directory.join(JOURNAL), MAX_META)?;
    let journal: Journal =
        serde_json::from_slice(&bytes).map_err(|_| EditError::RecoveryCorrupt)?;
    if journal.version != 1 {
        return Err(EditError::Unsupported("journal schema"));
    }
    if journal.revision == 0 {
        return Err(EditError::RecoveryCorrupt);
    }
    let base = directory.join("project");
    // Canonical list membership rejects traversal, links and unbounded filenames.
    let paths = project::autosave::list(&base)?;
    let path = paths
        .into_iter()
        .find(|p| p.file_name().and_then(|n| n.to_str()) == Some(journal.snapshot.as_str()))
        .ok_or(EditError::RecoveryCorrupt)?;
    let bytes = read_bounded(&path, MAX_PROJECT)?;
    if *blake3::hash(&bytes).as_bytes() != journal.content_hash {
        return Err(EditError::RecoveryCorrupt);
    }
    let project = project::decode(&bytes)?.project;
    if project.data().id != journal.project_id || project.data().name.as_str() != journal.name {
        return Err(EditError::RecoveryCorrupt);
    }
    Ok(Candidate {
        directory,
        _lock: file,
        journal,
        project,
    })
}
/// Scan the app-provided private recovery root, capped at 10,000 entries. Live
/// sessions are excluded, including other windows in this process. A candidate
/// remains exclusively claimed while the returned handle exists.
pub fn discover(root: &Path) -> Result<Discovery> {
    let mut result = Discovery {
        candidates: Vec::new(),
        diagnostics: Vec::new(),
    };
    if !root.exists() {
        return Ok(result);
    }
    if !fs::symlink_metadata(root)?.file_type().is_dir() {
        return Err(EditError::RecoveryCorrupt);
    }
    for (i, entry) in fs::read_dir(root)?.enumerate() {
        if i >= 10000 {
            return Err(EditError::Limit);
        }
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let directory = entry.path();
        let found = (|| {
            let Some(file) = lock(&directory.join("lock"))? else {
                return Ok(None);
            };
            if !directory.join(JOURNAL).try_exists()? {
                return Ok(None);
            }
            candidate(directory, file).map(Some)
        })();
        match found {
            Ok(Some(c)) => result.candidates.push(c),
            Ok(None) => {}
            Err(e) => result.diagnostics.push(e),
        }
    }
    Ok(result)
}
fn clear(directory: &Path) -> Result<()> {
    // Descriptor first: interruption cannot resurrect a discarded candidate.
    match fs::remove_file(directory.join(JOURNAL)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    for path in project::autosave::list(&directory.join("project"))? {
        fs::remove_file(path)?;
    }
    Ok(())
}
fn publish(
    directory: &Path,
    project: &Project,
    saved_path: Option<PathBuf>,
    revision: u64,
) -> Result<()> {
    // Stop permanently on any failure. With two snapshots, even failure between
    // snapshot and descriptor publication preserves the preceding committed pair.
    let path = project::autosave::save(&directory.join("project"), project, 2)?;
    let bytes = read_bounded(&path, MAX_PROJECT)?;
    let journal = Journal {
        version: 1,
        project_id: project.data().id,
        name: project.data().name.as_str().to_owned(),
        saved_path,
        revision,
        content_hash: *blake3::hash(&bytes).as_bytes(),
        timestamp_unix_ms: u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| EditError::Invalid("system clock"))?
                .as_millis(),
        )
        .map_err(|_| EditError::Limit)?,
        snapshot: path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(EditError::RecoveryCorrupt)?
            .to_owned(),
    };
    let bytes = serde_json::to_vec(&journal)
        .map_err(|_| EditError::Unsupported("journal path encoding"))?;
    if bytes.len() as u64 > MAX_META {
        return Err(EditError::Limit);
    }
    project::atomic_write(&directory.join(JOURNAL), &bytes)?;
    Ok(())
}
// The runner accepts reusable Fn tasks. Take ownership for IO, returning it on
// success, error or unwind without holding a state mutex across filesystem calls.
struct SaveWork {
    source: Option<WriterLease>,
    writer: WriterLease,
    receipt: Option<SaveReceipt>,
}
type SaveSlot = Arc<Mutex<Option<SaveWork>>>;
struct WorkingSave {
    slot: SaveSlot,
    work: Option<SaveWork>,
}
impl Drop for WorkingSave {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.slot.lock() {
            *slot = self.work.take();
        }
    }
}
fn save_on_worker(slot: &SaveSlot, project: &Project, metadata: &project::Metadata) -> Result<()> {
    let work = slot
        .lock()
        .map_err(|_| EditError::JobPoisoned)?
        .take()
        .ok_or(EditError::RecoveryBusy)?;
    let mut active = WorkingSave {
        slot: slot.clone(),
        work: Some(work),
    };
    let work = active.work.as_mut().ok_or(EditError::RecoveryBusy)?;
    work.receipt = Some(if let Some(source) = &work.source {
        source.save_to(&mut work.writer, project)?
    } else {
        work.writer.save(project)?
    });
    work.writer.save_metadata(metadata)?;
    Ok(())
}
struct SaveFlight {
    slot: SaveSlot,
    in_place: bool,
}
struct Flight {
    id: JobId,
    target: JobTarget,
    revision: u64,
    snapshot: Arc<Project>,
    save: Option<SaveFlight>,
    clear: bool,
}
// Admission refusals precede a persistence job and can be corrected by choosing
// another destination. Worker/publication failures must remain sticky.
enum PumpFailure {
    Admission(EditError),
    Persistence(EditError),
}
impl From<EditError> for PumpFailure {
    fn from(error: EditError) -> Self {
        Self::Persistence(error)
    }
}
/// One active Session, one journal, one in-flight persistence job and one latest
/// pending snapshot. Drop joins workers but deliberately performs no implicit save.
pub struct Autosave {
    session: Session,
    // Runner is dropped/joined before the session lock is released.
    runner: JobRunner,
    budget: ByteBudget,
    directory: PathBuf,
    _lock: File,
    clock: Arc<dyn Clock>,
    schedule: Schedule,
    writer: Option<WriterLease>,
    saved_path: Option<PathBuf>,
    saved_receipt: Option<SaveReceipt>,
    saved: Option<Arc<Project>>,
    observed: Arc<Project>,
    revision: u64,
    journaled: u64,
    first_dirty: Option<Duration>,
    last_change: Duration,
    flight: Option<Flight>,
    pending_save: Option<PathBuf>,
    failure: Option<Arc<EditError>>,
    #[cfg(test)]
    fault: Option<fn() -> Result<()>>,
}
impl Autosave {
    /// Never-saved or recovered work; save() requires an explicit save-as.
    /// Root follows the app's private data-directory policy.
    pub fn new(
        root: &Path,
        session: Session,
        budget: ByteBudget,
        clock: Arc<dyn Clock>,
        schedule: Schedule,
    ) -> Result<Self> {
        Self::with_writer(root, session, None, budget, clock, schedule)
    }
    /// Acquire the project's writer lease before load and retain it through edits,
    /// background publication and acknowledgement. Busy/failed acquisition never
    /// exposes an editable Session. IO here is blocking; open off the UI thread.
    pub fn open(
        root: &Path,
        path: &Path,
        history_limit: usize,
        budget: ByteBudget,
        clock: Arc<dyn Clock>,
        schedule: Schedule,
    ) -> Result<Self> {
        let writer = WriterLease::acquire(path)?;
        let mut session = Session::new(writer.load()?.project, history_limit)?;
        session.restore_metadata(writer.metadata())?;
        Self::with_writer(root, session, Some(writer), budget, clock, schedule)
    }
    fn with_writer(
        root: &Path,
        session: Session,
        writer: Option<WriterLease>,
        budget: ByteBudget,
        clock: Arc<dyn Clock>,
        schedule: Schedule,
    ) -> Result<Self> {
        let saved_path = writer.as_ref().map(|w| w.path().to_owned());
        if schedule.debounce.is_zero()
            || schedule.max_dirty_age < schedule.debounce
            || schedule.max_dirty_age > Duration::from_secs(600)
        {
            return Err(EditError::Limit);
        }
        if saved_path.as_ref().is_some_and(|p| !p.is_absolute()) {
            return Err(EditError::Invalid("save path must be absolute"));
        }
        fs::create_dir_all(root)?;
        if !fs::symlink_metadata(root)?.file_type().is_dir() {
            return Err(EditError::RecoveryCorrupt);
        }
        // Atomic mkdir avoids timestamp/pid reuse and never adopts an old directory.
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let directory = loop {
            let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = root.join(format!("session-{}-{n}", std::process::id()));
            match fs::create_dir(&dir) {
                Ok(()) => break dir,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        };
        let file = File::options()
            .create_new(true)
            .read(true)
            .write(true)
            .open(directory.join("lock"))?;
        file.try_lock().map_err(|e| match e {
            TryLockError::Error(e) => lock_error(e),
            TryLockError::WouldBlock => EditError::RecoveryBusy,
        })?;
        let runner = JobRunner::new(
            RunnerConfig {
                concurrency: [1, 1, 1],
                queue: 1,
                retained: 1,
                network_attempts: 1,
                backoff: Duration::from_secs(1),
                progress_interval: Duration::from_millis(10),
            },
            budget.clone(),
            clock.clone(),
        )?;
        let observed = session.project().clone();
        let now = clock.now();
        let saved = saved_path.as_ref().map(|_| observed.clone());
        let first_dirty = saved_path.is_none().then_some(now);
        Ok(Self {
            session,
            runner,
            budget,
            directory,
            _lock: file,
            clock,
            schedule,
            writer,
            saved_path,
            saved_receipt: None,
            saved,
            observed,
            revision: 1,
            journaled: 0,
            first_dirty,
            last_change: now,
            flight: None,
            pending_save: None,
            failure: None,
            #[cfg(test)]
            fault: None,
        })
    }
    pub fn session(&self) -> &Session {
        &self.session
    }
    /// Dispatch edits/gestures here; use switch() to replace the active project.
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }
    pub fn saved_path(&self) -> Option<&Path> {
        self.saved_path.as_deref()
    }
    /// Receipt of the last acknowledged save, absent after open or stale completion.
    pub fn saved_receipt(&self) -> Option<&SaveReceipt> {
        self.saved_receipt.as_ref()
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn journaled_revision(&self) -> u64 {
        self.journaled
    }
    pub fn is_dirty(&self) -> bool {
        self.saved
            .as_ref()
            .is_none_or(|s| !Arc::ptr_eq(s, self.session.project()))
    }
    pub fn saving(&self) -> bool {
        self.pending_save.is_some() || self.flight.as_ref().is_some_and(|f| f.save.is_some())
    }
    fn observe(&mut self) -> Result<()> {
        if self.session.transaction_active() {
            return Ok(());
        }
        let now = self.clock.now();
        if !Arc::ptr_eq(&self.observed, self.session.project()) {
            self.revision = self.revision.checked_add(1).ok_or(EditError::Limit)?;
            self.observed = self.session.project().clone();
            self.last_change = now;
        }
        if self.is_dirty()
            && self.revision != self.journaled
            && self
                .flight
                .as_ref()
                .is_none_or(|f| f.revision != self.revision || f.clear)
        {
            self.first_dirty.get_or_insert(now);
        }
        Ok(())
    }
    /// Nonblocking event-loop pump. A failed job is sticky: no later publication
    /// can rotate away the last confirmed snapshot. Errors must be presented.
    pub fn tick(&mut self) -> Result<()> {
        if let Some(e) = &self.failure {
            return Err(EditError::PersistenceFailed(e.clone()));
        }
        match self.pump() {
            Ok(()) => Ok(()),
            Err(PumpFailure::Admission(error)) => Err(error),
            Err(PumpFailure::Persistence(EditError::StaleResult)) => Err(EditError::StaleResult),
            Err(PumpFailure::Persistence(error)) => {
                let error = Arc::new(error);
                self.failure = Some(error.clone());
                Err(EditError::PersistenceFailed(error))
            }
        }
    }
    fn pump(&mut self) -> std::result::Result<(), PumpFailure> {
        self.observe()?;
        self.runner.tick()?;
        self.runner.drain_events();
        if let Some(f) = &self.flight {
            let state = self
                .runner
                .snapshot(f.id)
                .ok_or(EditError::WorkerPanicked)?;
            match state.state {
                JobState::Failed | JobState::Cancelled => {
                    return Err(EditError::PersistenceFailed(
                        state
                            .error
                            .unwrap_or_else(|| Arc::new(EditError::WorkerPanicked)),
                    )
                    .into());
                }
                JobState::Completed => {
                    let f = self.flight.take().ok_or(EditError::WorkerPanicked)?;
                    self.runner
                        .take_output(f.id)
                        .ok_or(EditError::WorkerPanicked)?;
                    self.journaled = if f.clear { 0 } else { f.revision };
                    if self.revision == f.revision {
                        self.first_dirty = None;
                    }
                    if let Some(save) = f.save {
                        let mut work = save
                            .slot
                            .lock()
                            .map_err(|_| EditError::JobPoisoned)?
                            .take()
                            .ok_or(EditError::WorkerPanicked)?;
                        let receipt = work.receipt.take().ok_or(EditError::WorkerPanicked)?;
                        if receipt.path() != work.writer.path() {
                            return Err(EditError::StaleResult.into());
                        }
                        // An in-place save advances the held baseline even if the
                        // edited revision is now stale. Save-as retains its source
                        // lease until this revision can adopt the new destination.
                        let current = self.session.matches_target(&f.target)
                            && !self.session.transaction_active();
                        if save.in_place || current {
                            self.writer = Some(work.writer);
                        } else {
                            self.writer = work.source;
                        }
                        // Revision identity includes gesture rollback/save boundaries,
                        // not just equal content or a matching numeric revision.
                        if current {
                            self.session.saved()?;
                            self.saved_path = Some(receipt.path().to_owned());
                            self.saved_receipt = Some(receipt);
                            self.saved = Some(f.snapshot);
                        } else {
                            // The destination was written even though this revision
                            // cannot be acknowledged. Undo may restore the old saved
                            // Arc, which no longer proves equality with disk. Treat
                            // all stale destinations conservatively (including path
                            // aliases); only a fresh save may reestablish cleanliness.
                            self.saved = None;
                            self.saved_receipt = None;
                            return Err(EditError::StaleResult.into());
                        }
                    }
                }
                _ => {}
            }
        }
        if self.flight.is_none()
            && self.pending_save.is_some()
            && !self.session.transaction_active()
        {
            let path = self.pending_save.take().ok_or(EditError::RecoveryBusy)?;
            if let Err(error) = self.start(Some(path)) {
                return Err(if self.flight.is_none() {
                    PumpFailure::Admission(error)
                } else {
                    PumpFailure::Persistence(error)
                });
            }
        }
        if self.flight.is_none()
            && !self.session.transaction_active()
            && !self.is_dirty()
            && self.journaled != 0
        {
            self.start_task(None, true)?;
        }
        if self.flight.is_none()
            && !self.session.transaction_active()
            && self.is_dirty()
            && self.revision != self.journaled
        {
            let now = self.clock.now();
            if self
                .first_dirty
                .is_some_and(|first| now.saturating_sub(first) >= self.schedule.max_dirty_age)
                || now.saturating_sub(self.last_change) >= self.schedule.debounce
            {
                self.start(None)?;
            }
        }
        Ok(())
    }
    fn start(&mut self, save_path: Option<PathBuf>) -> Result<()> {
        self.start_task(save_path, false)
    }
    fn start_task(&mut self, save_path: Option<PathBuf>, clear_journal: bool) -> Result<()> {
        if self.flight.is_some() {
            return Err(EditError::RecoveryBusy);
        }
        if self.session.transaction_active() {
            return Err(EditError::TransactionActive);
        }
        let snapshot = self.session.project().clone();
        let mut metadata = self.session.metadata().clone();
        metadata.touch();
        let project = snapshot.clone();
        let directory = self.directory.clone();
        let old_path = self.saved_path.clone();

        let revision = self.revision;
        let input = CacheKey::new(KeyInputs {
            kind: ArtifactKind::Preview,
            source: [0; 32],
            provider: "engine",
            model: "session-journal",
            version: "1",
            settings: &revision.to_le_bytes(),
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        #[cfg(test)]
        let fault = self.fault;
        let save = if let Some(path) = save_path {
            // Only explicit save-as acquires a new destination. Existing project
            // saves reuse the lease acquired before load; aliases follow its path.
            let in_place = self.writer.as_ref().is_some_and(|writer| {
                fs::canonicalize(&path).is_ok_and(|path| path == writer.path())
                    || path == writer.path()
            });
            let writer = if in_place {
                self.writer.take().ok_or(EditError::RecoveryBusy)?
            } else {
                WriterLease::acquire(&path)?
            };
            Some(SaveFlight {
                slot: Arc::new(Mutex::new(Some(SaveWork {
                    source: if in_place { None } else { self.writer.take() },
                    writer,
                    receipt: None,
                }))),
                in_place,
            })
        } else {
            None
        };
        let slot = save.as_ref().map(|save| save.slot.clone());
        let submitted = self.runner.submit(
            JobSpec {
                kind: JobKind::Persist,
                class: ResourceClass::Cpu,
                input,
                bytes: JobBytes {
                    compositor: WORK_BYTES,
                    ..Default::default()
                },
            },
            move |ctx| {
                ctx.check_cancelled()?;
                #[cfg(test)]
                if let Some(fault) = fault {
                    fault()?;
                }
                if clear_journal {
                    clear(&directory)?;
                } else {
                    publish(&directory, &project, old_path.clone(), revision)?;
                }
                if let Some(slot) = &slot {
                    save_on_worker(slot, &project, &metadata)?;
                }
                Ok(Vec::new())
            },
        );
        let (id, _) = match submitted {
            Ok(submitted) => submitted,
            Err(error) => {
                if let Some(save) = save {
                    let work = save
                        .slot
                        .lock()
                        .map_err(|_| EditError::JobPoisoned)?
                        .take()
                        .ok_or(EditError::WorkerPanicked)?;
                    self.writer = if save.in_place {
                        Some(work.writer)
                    } else {
                        work.source
                    };
                }
                return Err(error);
            }
        };
        self.flight = Some(Flight {
            id,
            target: self.session.job_target(),
            revision,
            snapshot,
            save,
            clear: clear_journal,
        });
        self.first_dirty = None;
        self.runner.tick()?;
        Ok(())
    }
    /// Reserve the destination, then queue journal/project publication off-thread.
    /// Native path resolution/lease acquisition can block this caller. The journal
    /// precedes the project write. Poll tick until saving() is false; leaving before
    /// acknowledgement is not a successful save boundary.
    pub fn save_as(&mut self, path: PathBuf) -> Result<()> {
        if !path.is_absolute() {
            return Err(EditError::Invalid("save path must be absolute"));
        }
        // WriterLease resolves existing symlinks before publication. Apply the
        // storage boundary to that destination, not merely the supplied parent.
        let destination = match path.canonicalize() {
            Ok(destination) => destination,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => path.clone(),
            Err(error) => return Err(error.into()),
        };
        let parent = destination
            .parent()
            .ok_or(EditError::Invalid("save destination"))?
            .canonicalize()?;
        let root = self
            .directory
            .parent()
            .ok_or(EditError::RecoveryCorrupt)?
            .canonicalize()?;
        if parent.starts_with(root) {
            return Err(EditError::Invalid(
                "save destination is inside recovery storage",
            ));
        }
        self.tick()?;
        if self.saving() {
            return Err(EditError::RecoveryBusy);
        }
        if self.session.transaction_active() {
            return Err(EditError::TransactionActive);
        }
        self.pending_save = Some(path);
        self.tick()
    }
    /// Dispatch the registry save command; completion is acknowledged by tick().
    pub fn execute(&mut self, id: &str, params: serde_json::Value) -> Result<crate::Outcome> {
        if id == "project.save" {
            if params != serde_json::json!({}) {
                return Err(EditError::InvalidParams);
            }
            self.save()?;
            Ok(crate::Outcome {
                changed: false,
                created: Vec::new(),
            })
        } else {
            self.session.execute(id, params)
        }
    }
    pub fn save(&mut self) -> Result<()> {
        let path = self.saved_path.clone().ok_or(EditError::SaveAsRequired)?;
        self.save_as(path)
    }
    /// Acknowledge the old pending revision before replacing the active project.
    /// On failure this retains the current Session and refuses the switch.
    pub fn switch(&mut self, session: Session) -> Result<()> {
        self.flush()?;
        let root = self.directory.parent().ok_or(EditError::RecoveryCorrupt)?;
        let next = Self::new(
            root,
            session,
            self.budget.clone(),
            self.clock.clone(),
            self.schedule,
        )?;
        *self = next;
        Ok(())
    }
    /// Explicit blocking project-switch/exit barrier. IO remains on the job worker;
    /// the app should keep its progress presentation responsive while waiting.
    /// Failure refuses the boundary; Drop alone never claims to have flushed.
    pub fn flush(&mut self) -> Result<u64> {
        if self.session.transaction_active() {
            return Err(EditError::TransactionActive);
        }
        self.tick()?;
        while self.flight.is_some() {
            std::thread::sleep(Duration::from_millis(1));
            self.tick()?;
        }
        if self.is_dirty() && self.journaled != self.revision {
            self.start(None)?;
            while self.flight.is_some() {
                std::thread::sleep(Duration::from_millis(1));
                self.tick()?;
            }
        }
        Ok(self.journaled)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "injected worker failures and test fixtures"
)]
mod tests {
    use super::*;
    use crate::jobs::SystemClock;
    fn open(root: &Path) -> Autosave {
        Autosave::new(
            root,
            Session::new(
                Project::empty(project::ProjectId(1), "".into()).unwrap(),
                10,
            )
            .unwrap(),
            ByteBudget::new(64 * 1024 * 1024).unwrap(),
            Arc::new(SystemClock::default()),
            Schedule::default(),
        )
        .unwrap()
    }
    #[test]
    fn worker_lease_transfer_survives_unwind_without_poisoning_the_slot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.vibecut");
        let writer = WriterLease::acquire(&path).unwrap();
        let slot = Arc::new(Mutex::new(Some(SaveWork {
            source: None,
            writer,
            receipt: None,
        })));
        let unwound = std::panic::catch_unwind(|| {
            let work = slot.lock().unwrap().take();
            let _active = WorkingSave {
                slot: slot.clone(),
                work,
            };
            panic!("injected after lease transfer");
        });
        assert!(unwound.is_err());
        assert!(slot.lock().unwrap().is_some());
        assert!(matches!(
            WriterLease::acquire(&path),
            Err(project::Error::Busy)
        ));
        drop(slot);
        assert!(WriterLease::acquire(&path).is_ok());
    }
    #[test]
    fn disk_full_and_worker_unwind_remain_typed_and_preserve_published_work() {
        for fault in [disk_full as fn() -> Result<()>, unwind] {
            let dir = tempfile::tempdir().unwrap();
            let mut a = open(dir.path());
            a.flush().unwrap();
            let before = fs::read(a.directory.join(JOURNAL)).unwrap();
            a.session_mut()
                .apply(crate::Command::AddTrack {
                    name: "".into(),
                    kind: project::TrackKind::Overlay,
                })
                .unwrap();
            a.fault = Some(fault);
            let error = a.flush().unwrap_err();
            fn cause(e: &EditError) -> &EditError {
                match e {
                    EditError::PersistenceFailed(e) => cause(e),
                    other => other,
                }
            }
            assert!(matches!(
                cause(&error),
                EditError::Io(_) | EditError::WorkerPanicked
            ));
            assert!(a.is_dirty());
            assert!(a.tick().is_err());
            assert!(
                a.switch(
                    Session::new(
                        Project::empty(project::ProjectId(2), "".into()).unwrap(),
                        10
                    )
                    .unwrap()
                )
                .is_err()
            );
            assert_eq!(fs::read(a.directory.join(JOURNAL)).unwrap(), before);
            drop(a);
            assert_eq!(discover(dir.path()).unwrap().candidates.len(), 1);
        }
    }
    fn disk_full() -> Result<()> {
        Err(std::io::Error::from(std::io::ErrorKind::StorageFull).into())
    }
    fn unwind() -> Result<()> {
        panic!("injected persistence unwind")
    }
    #[test]
    fn unsupported_lock_is_not_misclassified_as_a_dead_session() {
        assert!(matches!(
            lock_error(std::io::ErrorKind::Unsupported.into()),
            EditError::Unsupported("session locking")
        ));
    }
    #[test]
    fn descriptor_failure_preserves_last_confirmed_pair() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = open(dir.path());
        a.flush().unwrap();
        let before = fs::read(a.directory.join(JOURNAL)).unwrap();
        // A descriptor publication failure after a complete snapshot must not
        // retire the previous descriptor's project. Test the exact write boundary.
        let later = Project::empty(project::ProjectId(2), "new".into()).unwrap();
        project::autosave::save(&a.directory.join("project"), &later, 2).unwrap();
        assert_eq!(fs::read(a.directory.join(JOURNAL)).unwrap(), before);
        drop(a);
        let d = discover(dir.path()).unwrap();
        assert_eq!(
            d.candidates.first().unwrap().journal.project_id,
            project::ProjectId(1)
        );
    }
}
