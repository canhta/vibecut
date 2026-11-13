//! Owns recoverable persistent sessions.
use crate::cache::{ArtifactKind, CacheKey, KeyInputs};
use crate::jobs::{
    Clock, JobBytes, JobId, JobKind, JobRunner, JobSpec, JobState, ResourceClass, RunnerConfig,
};
use crate::session::wire::{ErrorCode, Failure};
use crate::{ByteBudget, JobTarget, Session};
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
const JOURNAL_VERSION: u32 = 2;

/// Recovery diagnostics retain typed causes without exposing project content.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Edit(#[from] crate::session::EditError),
    #[error(transparent)]
    Runtime(#[from] crate::runtime::Error),
    #[error("recovery candidate is corrupt or incomplete")]
    Corrupt,
    #[error("persistence operation is already active")]
    Busy,
    #[error("recovered or never-saved work requires save-as")]
    SaveAsRequired,
    #[error("session persistence failed")]
    PersistenceFailed(#[source] Arc<dyn Failure + 'static>),
    #[error("project candidate rejected")]
    Project(#[from] project::Error),
    #[error("job/cache IO failed")]
    Io(#[from] std::io::Error),
    #[error("command limit exceeded")]
    Limit,
    #[error("invalid edit: {0}")]
    Invalid(&'static str),
    #[error("unsupported edit: {0}")]
    Unsupported(&'static str),
}
impl Failure for Error {
    fn code(&self) -> ErrorCode {
        use ErrorCode as C;
        match self {
            Self::Edit(error) => error.code(),
            Self::Runtime(error) => error.code(),
            Self::PersistenceFailed(cause) => cause.code(),
            Self::Limit => C::Limit,
            Self::Io(_) | Self::Project(project::Error::Io(_)) => C::Io,
            Self::Project(project::Error::Busy) => C::Busy,
            Self::Project(project::Error::Conflict) => C::Conflict,
            Self::Project(project::Error::Media(_)) => C::Media,
            Self::Corrupt
            | Self::Busy
            | Self::SaveAsRequired
            | Self::Invalid(_)
            | Self::Unsupported(_)
            | Self::Project(_) => C::Refused,
        }
    }
    fn transient(&self) -> bool {
        match self {
            Self::Edit(error) => error.transient(),
            Self::Runtime(error) => error.transient(),
            _ => false,
        }
    }
    fn job_policy(&self) -> crate::session::wire::JobFailurePolicy {
        match self {
            Self::Edit(error) => error.job_policy(),
            Self::Runtime(error) => error.job_policy(),
            _ => crate::session::wire::JobFailurePolicy::default(),
        }
    }
}
impl From<crate::session::wire::ControlError> for Error {
    fn from(error: crate::session::wire::ControlError) -> Self {
        Self::Edit(error.into())
    }
}
impl From<crate::session::SessionError> for Error {
    fn from(error: crate::session::SessionError) -> Self {
        Self::Edit(error.into())
    }
}
pub type Result<T> = std::result::Result<T, Error>;

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
    _lock: SessionLock,
    pub journal: Journal,
    project: Project,
}
/// Corrupt candidates have a typed diagnostic; lock/IO failures retain their source.
pub struct Discovery {
    pub candidates: Vec<Candidate>,
    pub diagnostics: Vec<Error>,
}
struct SessionLock(File);
// The runner accepts reusable Fn tasks. Take ownership for IO, returning it on
// success, error or unwind without holding a state mutex across filesystem calls.
struct SaveWork {
    source: Option<WriterLease>,
    writer: WriterLease,
    receipt: Option<SaveReceipt>,
    metadata: Option<project::Metadata>,
}
type SaveSlot = Arc<Mutex<Option<SaveWork>>>;
struct WorkingSave {
    slot: SaveSlot,
    work: Option<SaveWork>,
}
struct SaveFlight {
    destination: PathBuf,
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
    Admission(Error),
    Persistence(Error),
}
/// One active Session, one journal, one in-flight persistence job and one latest
/// pending snapshot. Drop joins workers but deliberately performs no implicit save.
pub struct Autosave {
    session: Session,
    // Runner is dropped/joined before the session lock is released.
    runner: JobRunner,
    budget: ByteBudget,
    directory: PathBuf,
    _lock: SessionLock,
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
    failure: Option<Arc<dyn Failure + 'static>>,
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
}

impl Autosave {
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
        let writer = WriterLease::acquire(path).map_err(crate::session::EditError::from)?;
        let mut session = Session::new(
            writer
                .load()
                .map_err(crate::session::EditError::from)?
                .project,
            history_limit,
        )?;
        let mut metadata = writer.metadata();
        if matches!(metadata.thumbnail, project::Thumbnail::Missing) {
            metadata.thumbnail = crate::store::project_cover::capture(
                session.project(),
                &crate::ByteBudgetHooks::new(&budget, None),
            );
            // Optional thumbnail failure must never prevent opening editable content.
            let _ = writer.save_metadata(&metadata);
        }
        session.restore_metadata(metadata)?;
        Self::with_writer(root, session, Some(writer), budget, clock, schedule)
    }
}

impl Autosave {
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
            return Err(Error::from(crate::session::wire::ControlError::Limit));
        }
        if saved_path.as_ref().is_some_and(|p| !p.is_absolute()) {
            return Err(Error::Edit(crate::session::EditError::Invalid(
                "save path must be absolute",
            )));
        }
        fs::create_dir_all(root)?;
        if !fs::symlink_metadata(root)?.file_type().is_dir() {
            return Err(Error::Corrupt);
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
            TryLockError::WouldBlock => Error::Busy,
        })?;
        let file = SessionLock(file);
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
}

impl Autosave {
    pub fn session(&self) -> &Session {
        &self.session
    }
}

impl Autosave {
    /// Dispatch edits/gestures here; use switch() to replace the active project.
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }
}

impl Autosave {
    pub fn saved_path(&self) -> Option<&Path> {
        self.saved_path.as_deref()
    }
}

impl Autosave {
    /// Snapshot of active writer and recovery paths for external publication admission.
    /// Includes an in-flight Save As destination even while its lease is on a worker.
    pub fn protected_export_paths(&self) -> Result<Vec<PathBuf>> {
        let mut paths = vec![self.directory.clone()];
        for path in self
            .saved_path
            .iter()
            .chain(self.pending_save.iter())
            .chain(
                self.flight
                    .as_ref()
                    .and_then(|flight| flight.save.as_ref())
                    .map(|save| &save.destination),
            )
        {
            paths.push(path.clone());
            paths.push(project::metadata_path(path)?);
        }
        Ok(paths)
    }
}

impl Autosave {
    /// Receipt of the last acknowledged save, absent after open or stale completion.
    pub fn saved_receipt(&self) -> Option<&SaveReceipt> {
        self.saved_receipt.as_ref()
    }
}

impl Autosave {
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

impl Autosave {
    pub fn journaled_revision(&self) -> u64 {
        self.journaled
    }
}

impl Autosave {
    pub fn is_dirty(&self) -> bool {
        self.saved
            .as_ref()
            .is_none_or(|s| !Arc::ptr_eq(s, self.session.project()))
    }
}

impl Autosave {
    pub fn saving(&self) -> bool {
        self.pending_save.is_some()
            || self
                .flight
                .as_ref()
                .is_some_and(|f| f.save.is_some() || f.clear)
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
    #[cfg(unix)]
    #[test]
    fn dropped_session_and_candidate_release_inherited_locks() {
        for claimed in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut autosave = open(dir.path());
            let revision = autosave.flush().unwrap();
            // Pin the shared open-file description as a concurrent fork can
            // before exec, without depending on subprocess scheduling.
            let inherited = if claimed {
                drop(autosave);
                let candidate = discover(dir.path()).unwrap().candidates.pop().unwrap();
                let inherited = candidate._lock.0.try_clone().unwrap();
                assert!(discover(dir.path()).unwrap().candidates.is_empty());
                drop(candidate);
                inherited
            } else {
                let inherited = autosave._lock.0.try_clone().unwrap();
                assert!(discover(dir.path()).unwrap().candidates.is_empty());
                drop(autosave);
                inherited
            };
            let discovery = discover(dir.path()).unwrap();
            assert!(discovery.diagnostics.is_empty());
            assert_eq!(discovery.candidates.len(), 1, "claimed: {claimed}");
            assert_eq!(discovery.candidates[0].journal.revision, revision);
            drop(inherited);
            assert!(discover(dir.path()).unwrap().candidates.is_empty());
            drop(discovery);
            assert_eq!(discover(dir.path()).unwrap().candidates.len(), 1);
        }
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
            metadata: None,
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
        use crate::control::{self, Caller, Payload, Policy, ResponseValue};
        struct Allow;
        impl Policy for Allow {
            fn authorize(&self, _: &crate::Command, _: &Project) -> crate::control::Result<()> {
                Ok(())
            }
        }
        let caller = || Caller {
            principal: "recovery-test",
            external: true,
            policy: &Allow,
            fonts: None,
            sounds: None,
            music: None,
            brand: None,
            media: None,
            chapters: None,
            captions: None,
            vision: None,
        };
        for (fault, code) in [
            (disk_full as fn() -> Result<()>, ErrorCode::Io),
            (unwind, ErrorCode::WorkerFailure),
            (cancelled, ErrorCode::Cancelled),
            (worker_stale, ErrorCode::StaleResult),
            (network_transient, ErrorCode::Job),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut a = open(dir.path());
            a.flush().unwrap();
            let before = fs::read(a.directory.join(JOURNAL)).unwrap();
            let journal: Journal = serde_json::from_slice(&before).unwrap();
            let confirmed_snapshot = project::autosave::list(&a.directory.join("project"))
                .unwrap()
                .into_iter()
                .find(|path| {
                    path.file_name().and_then(|name| name.to_str())
                        == Some(journal.snapshot.as_str())
                })
                .unwrap();
            let confirmed_bytes = fs::read(&confirmed_snapshot).unwrap();
            a.session_mut()
                .apply(crate::Command::AddTrack {
                    placement: crate::TrackPlacement::Auto,
                    name: "".into(),
                    kind: project::TrackKind::Video,
                })
                .unwrap();
            a.fault = Some(fault);
            let error = a.flush().unwrap_err();
            let Error::PersistenceFailed(raw) = &error else {
                panic!("missing persistence envelope")
            };
            let original = raw.as_ref() as &(dyn std::error::Error + 'static);
            let exposed = <Error as std::error::Error>::source(&error)
                .unwrap()
                .downcast_ref::<Arc<dyn Failure>>()
                .unwrap();
            assert!(Arc::ptr_eq(raw, exposed));
            if let Some(error) = original.downcast_ref::<crate::RuntimeError>() {
                assert!(matches!(error, crate::RuntimeError::WorkerPanicked));
            } else {
                match original.downcast_ref::<Error>().unwrap() {
                    Error::Io(io) => {
                        assert_eq!(io.kind(), std::io::ErrorKind::StorageFull);
                        assert_eq!(raw.to_string(), "job/cache IO failed");
                        assert!(std::ptr::eq(
                            io,
                            raw.source()
                                .unwrap()
                                .downcast_ref::<std::io::Error>()
                                .unwrap()
                        ));
                    }
                    Error::Runtime(crate::RuntimeError::Cancelled) => {
                        assert_eq!(code, ErrorCode::Cancelled)
                    }
                    Error::Runtime(crate::RuntimeError::NetworkTransient) => {
                        assert_eq!(code, ErrorCode::Job);
                        assert!(raw.transient());
                    }
                    Error::Edit(crate::session::EditError::Session(
                        crate::session::SessionError::StaleResult,
                    )) => assert_eq!(code, ErrorCode::StaleResult),
                    _ => panic!("lost the typed worker cause"),
                }
            }
            let snapshot = a
                .runner
                .snapshots()
                .into_iter()
                .find(|row| row.error.is_some())
                .unwrap();
            assert!(Arc::ptr_eq(raw, snapshot.error.as_ref().unwrap()));
            assert_eq!(snapshot.attempts, 1);
            assert!(!error.job_policy().terminal_cancelled);
            assert_eq!(error.job_policy().retry_after_seconds, None);
            assert_eq!(error.code(), code);
            assert!(!error.transient());
            let shared = a.failure.as_ref().unwrap().clone();
            let Error::PersistenceFailed(repeated) = a.tick().unwrap_err() else {
                panic!("worker failures, including nested stale results, must remain sticky");
            };
            assert!(Arc::ptr_eq(raw, &shared));
            assert!(Arc::ptr_eq(&shared, &repeated));
            let Error::PersistenceFailed(flushed) = a.flush().unwrap_err() else {
                panic!("lost sticky cause")
            };
            assert!(Arc::ptr_eq(raw, &flushed));
            assert_eq!(a.budget.snapshot().unwrap().limited_used(), 0);

            // Feed the actual failed worker result through control admission,
            // receipt replay and event emission, without a second publication.
            let project = a.session.project().clone();
            let revision = a.session.revision();
            let history = a.session.history_len();
            let mut cursor = a.session.snapshot_and_subscribe(&a.runner).unwrap().cursor;
            let request = control::Request {
                api_version: control::API_VERSION,
                registry_hash: Some(control::registry_hash().into()),
                session_id: a.session.session_id().into(),
                request_id: "persistence-fault".into(),
                idempotency_key: Some("fault".into()),
                base_rev: Some(revision.clone()),
                command: "project.save".into(),
                params: serde_json::json!({}),
            };
            let response = a
                .session
                .control_lifecycle_save(request.clone(), caller(), |_, _| Err(error.into()))
                .unwrap();
            assert!(matches!(response.value, ResponseValue::Error(observed) if observed == code));
            let replay = a
                .session
                .control_lifecycle_save(request, caller(), |_, _| panic!("replayed publication"))
                .unwrap();
            assert!(matches!(replay.value, ResponseValue::Error(observed) if observed == code));
            assert_eq!(a.session.project(), &project);
            assert_eq!(a.session.revision(), revision);
            assert_eq!(a.session.history_len(), history);
            assert!(
                matches!(a.session.events(&mut cursor).unwrap().as_slice(), [control::Event { payload: Payload::CommandError { code: observed, .. }, .. }] if *observed == code)
            );
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
            assert_eq!(fs::read(&confirmed_snapshot).unwrap(), confirmed_bytes);
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
    fn cancelled() -> Result<()> {
        Err(Error::Runtime(crate::RuntimeError::Cancelled))
    }
    fn network_transient() -> Result<()> {
        Err(Error::Runtime(crate::RuntimeError::NetworkTransient))
    }
    fn worker_stale() -> Result<()> {
        Err(Error::from(crate::session::SessionError::StaleResult))
    }
    #[test]
    fn unsupported_lock_is_not_misclassified_as_a_dead_session() {
        assert!(matches!(
            lock_error(std::io::ErrorKind::Unsupported.into()),
            Error::Unsupported("session locking")
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
mod discovery;

mod checkpoint;

mod lifecycle;

use discovery::{clear, lock_error, read_bounded};

pub use discovery::discover;
