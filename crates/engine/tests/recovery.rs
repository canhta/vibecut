#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test fixtures and assertions"
)]
use engine::{
    ByteBudget, Command, EditError, Session,
    jobs::Clock,
    recovery::{Autosave, Schedule, discover},
};
use project::{Project, ProjectId, TrackKind};
use std::{
    fs,
    path::Path,
    process::{Child, Command as Process},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Default)]
struct TestClock(AtomicU64);
impl Clock for TestClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}
impl TestClock {
    fn set(&self, millis: u64) {
        self.0.store(millis, Ordering::SeqCst);
    }
}
fn session(id: u64) -> Session {
    Session::new(
        Project::empty(ProjectId(id), "Tiếng Việt".into()).unwrap(),
        10,
    )
    .unwrap()
}
fn open(root: &Path, id: u64, path: Option<std::path::PathBuf>, clock: Arc<TestClock>) -> Autosave {
    Autosave::new(
        root,
        session(id),
        path,
        ByteBudget::new(64 * 1024 * 1024).unwrap(),
        clock,
        Schedule::default(),
    )
    .unwrap()
}
fn edit(a: &mut Autosave) {
    a.session_mut()
        .apply(Command::AddTrack {
            name: "Track".into(),
            kind: TrackKind::Overlay,
        })
        .unwrap();
}
fn until(mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < end, "worker timed out");
        thread::sleep(Duration::from_millis(1));
    }
}
fn published(a: &mut Autosave) {
    until(|| {
        a.tick().unwrap();
        a.journaled_revision() == a.revision()
    });
}
fn saved(a: &mut Autosave) {
    until(|| {
        a.tick().unwrap();
        !a.saving()
    });
}

#[test]
fn debounce_coalesces_and_continuous_edits_have_a_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(TestClock::default());
    let mut a = open(dir.path(), 1, None, clock.clone());
    for millis in [0, 200, 400, 600] {
        clock.set(millis);
        edit(&mut a);
        a.tick().unwrap();
    }
    clock.set(1399);
    a.tick().unwrap();
    assert_eq!(a.journaled_revision(), 0);
    clock.set(1400);
    published(&mut a);
    assert_eq!(a.revision(), 5);
    // No idle period reaches 800ms, but the first dirty change bounds the burst.
    for millis in (1500..11500).step_by(100) {
        clock.set(millis);
        edit(&mut a);
        a.tick().unwrap();
    }
    assert_eq!(a.journaled_revision(), 5);
    clock.set(11500);
    edit(&mut a);
    a.tick().unwrap();
    until(|| {
        a.tick().unwrap();
        a.journaled_revision() > 5
    });
    let revision = a.flush().unwrap();
    drop(a);
    let discovery = discover(dir.path()).unwrap();
    assert!(discovery.diagnostics.is_empty());
    assert_eq!(discovery.candidates.len(), 1);
    assert_eq!(
        discovery.candidates.first().unwrap().journal.revision,
        revision
    );
}

#[test]
fn live_sessions_are_excluded_and_candidates_are_exclusively_claimed() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(TestClock::default());
    let mut a = open(dir.path(), 1, None, clock.clone());
    let mut b = open(dir.path(), 2, None, clock);
    a.flush().unwrap();
    b.flush().unwrap();
    assert!(discover(dir.path()).unwrap().candidates.is_empty());
    drop(a);
    let d = discover(dir.path()).unwrap();
    assert_eq!(d.candidates.len(), 1);
    assert_eq!(
        d.candidates.first().unwrap().journal.project_id,
        ProjectId(1)
    );
    assert!(discover(dir.path()).unwrap().candidates.is_empty());
    drop(d);
    drop(b);
    assert_eq!(discover(dir.path()).unwrap().candidates.len(), 2);
}

#[test]
fn recover_discard_and_save_as_preserve_original_then_promote() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("gốc project.vibecut");
    project::save(&original, session(1).project()).unwrap();
    let bytes = fs::read(&original).unwrap();
    let root = dir.path().join("recovery");
    let clock = Arc::new(TestClock::default());
    let mut a = open(&root, 1, Some(original.clone()), clock.clone());
    assert!(!a.is_dirty());
    edit(&mut a);
    a.flush().unwrap();
    drop(a);
    let mut d = discover(&root).unwrap();
    let candidate = d.candidates.pop().unwrap();
    assert_eq!(
        candidate.journal.saved_path.as_deref(),
        Some(original.as_path())
    );
    let recovered = candidate.recover(10).unwrap();
    let mut a = Autosave::new(
        &root,
        recovered,
        None,
        ByteBudget::new(64 * 1024 * 1024).unwrap(),
        clock,
        Schedule::default(),
    )
    .unwrap();
    assert!(matches!(a.save(), Err(EditError::SaveAsRequired)));
    let destination = dir.path().join("recovered.vibecut");
    a.save_as(destination.clone()).unwrap();
    saved(&mut a);
    assert!(!a.is_dirty());
    assert_eq!(a.saved_path(), Some(destination.as_path()));
    assert_eq!(
        project::load(&destination)
            .unwrap()
            .project
            .data()
            .timeline
            .tracks
            .len(),
        1
    );
    edit(&mut a);
    a.save().unwrap();
    saved(&mut a);
    a.flush().unwrap();
    assert!(!a.is_dirty());
    assert_eq!(
        project::load(&destination)
            .unwrap()
            .project
            .data()
            .timeline
            .tracks
            .len(),
        2
    );
    candidate.discard().unwrap();
    assert_eq!(fs::read(&original).unwrap(), bytes);
    drop(a);
    assert!(discover(&root).unwrap().candidates.is_empty());
}

#[test]
fn stale_save_completion_cannot_acknowledge_new_edits() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = open(
        &dir.path().join("recovery"),
        1,
        None,
        Arc::new(TestClock::default()),
    );
    a.save_as(dir.path().join("saved.vibecut")).unwrap();
    edit(&mut a); // Deterministic: completion cannot be acknowledged before tick.
    until(|| match a.tick() {
        Err(EditError::StaleResult) => true,
        Ok(()) => false,
        Err(e) => panic!("unexpected {e}"),
    });
    assert!(a.is_dirty());
    assert!(a.saved_path().is_none());
    a.flush().unwrap(); // Staleness must not stop recovery for the newer edits.
    drop(a);
    let d = discover(&dir.path().join("recovery")).unwrap();
    assert_eq!(
        d.candidates
            .first()
            .unwrap()
            .recover(10)
            .unwrap()
            .project()
            .data()
            .timeline
            .tracks
            .len(),
        1
    );
}

#[test]
fn switch_flushes_pending_and_gestures_cannot_cross_a_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = open(dir.path(), 1, None, Arc::new(TestClock::default()));
    edit(&mut a);
    a.session_mut().begin_gesture().unwrap();
    assert!(matches!(
        a.switch(session(2), None),
        Err(EditError::TransactionActive)
    ));
    a.session_mut().cancel_gesture().unwrap();
    a.switch(session(2), None).unwrap();
    let d = discover(dir.path()).unwrap();
    assert_eq!(d.candidates.len(), 1);
    let c = d.candidates.first().unwrap();
    assert_eq!(c.journal.project_id, ProjectId(1));
    assert_eq!(
        c.recover(10)
            .unwrap()
            .project()
            .data()
            .timeline
            .tracks
            .len(),
        1
    );
}

#[test]
fn corrupt_truncated_hash_mismatch_and_schema_are_diagnosed_without_deletion() {
    for mode in 0..4 {
        let dir = tempfile::tempdir().unwrap();
        let mut a = open(dir.path(), 1, None, Arc::new(TestClock::default()));
        a.flush().unwrap();
        drop(a);
        let directory = fs::read_dir(dir.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let meta = directory.join("journal.json");
        let original = fs::read(&meta).unwrap();
        match mode {
            0 => fs::write(&meta, b"{truncated").unwrap(),
            1 => {
                let snapshot = project::autosave::list(&directory.join("project"))
                    .unwrap()
                    .pop()
                    .unwrap();
                fs::write(snapshot, b"partial").unwrap();
            }
            2 => {
                let mut json: serde_json::Value = serde_json::from_slice(&original).unwrap();
                json["snapshot"] = serde_json::json!("../../original.vibecut");
                fs::write(&meta, serde_json::to_vec(&json).unwrap()).unwrap();
            }
            _ => {
                let mut json: serde_json::Value = serde_json::from_slice(&original).unwrap();
                json["version"] = serde_json::json!(2);
                fs::write(&meta, serde_json::to_vec(&json).unwrap()).unwrap();
            }
        }
        let before = fs::read(&meta).unwrap();
        let d = discover(dir.path()).unwrap();
        assert!(d.candidates.is_empty());
        assert_eq!(d.diagnostics.len(), 1);
        assert_eq!(fs::read(&meta).unwrap(), before);
    }
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn journal_survives_forced_process_termination_and_another_live_instance() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = ChildGuard(
        Process::new(std::env::current_exe().unwrap())
            .args(["--exact", "recovery_child", "--nocapture"])
            .env("VIBECUT_TEST_RECOVERY_ROOT", dir.path())
            .spawn()
            .unwrap(),
    );
    until(|| dir.path().join("ready").exists());
    let root = dir.path().join("journals");
    let mut other = open(&root, 2, None, Arc::new(TestClock::default()));
    other.flush().unwrap();
    assert!(discover(&root).unwrap().candidates.is_empty());
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let d = discover(&root).unwrap();
    assert!(d.diagnostics.is_empty());
    assert_eq!(d.candidates.len(), 1);
    let c = d.candidates.first().unwrap();
    assert_eq!(c.journal.revision, 2);
    assert_eq!(
        c.recover(10)
            .unwrap()
            .project()
            .data()
            .timeline
            .tracks
            .len(),
        1
    );
}
#[test]
fn recovery_child() {
    let Some(root) = std::env::var_os("VIBECUT_TEST_RECOVERY_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let mut a = open(
        &root.join("journals"),
        1,
        None,
        Arc::new(TestClock::default()),
    );
    a.flush().unwrap();
    edit(&mut a);
    assert_eq!(a.flush().unwrap(), 2);
    fs::write(root.join("ready"), b"published").unwrap();
    loop {
        thread::sleep(Duration::from_secs(1));
    }
}

#[test]
fn undo_to_saved_state_retires_obsolete_recovery_and_internal_save_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("recovery");
    let original = dir.path().join("original.vibecut");
    project::save(&original, session(1).project()).unwrap();
    let mut a = open(&root, 1, Some(original), Arc::new(TestClock::default()));
    edit(&mut a);
    a.flush().unwrap();
    a.session_mut().undo().unwrap();
    assert!(!a.is_dirty());
    a.flush().unwrap();
    let directory = fs::read_dir(&root).unwrap().next().unwrap().unwrap().path();
    assert!(matches!(
        a.save_as(directory.join("lock")),
        Err(EditError::Invalid(_))
    ));
    drop(a);
    assert!(discover(&root).unwrap().candidates.is_empty());
}
