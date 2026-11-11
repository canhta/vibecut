#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test fixtures and assertions"
)]
use engine::{
    ByteBudget, Command, EditError, Failure, Session,
    jobs::Clock,
    recovery::{Autosave, Error as RecoveryError, Schedule, discover},
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
    let budget = ByteBudget::new(64 * 1024 * 1024).unwrap();
    if let Some(path) = path {
        Autosave::open(root, &path, 10, budget, clock, Schedule::default()).unwrap()
    } else {
        Autosave::new(root, session(id), budget, clock, Schedule::default()).unwrap()
    }
}
fn edit(a: &mut Autosave) {
    a.session_mut()
        .apply(Command::AddTrack {
            placement: engine::TrackPlacement::Auto,
            name: "Track".into(),
            kind: TrackKind::Video,
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
    project::WriterLease::acquire(&original)
        .unwrap()
        .save(session(1).project())
        .unwrap();
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
        Some(original.canonicalize().unwrap().as_path())
    );
    let recovered = candidate.recover(10).unwrap();
    let mut a = Autosave::new(
        &root,
        recovered,
        ByteBudget::new(64 * 1024 * 1024).unwrap(),
        clock,
        Schedule::default(),
    )
    .unwrap();
    assert!(matches!(
        a.save(),
        Err(EditError::Recovery(RecoveryError::SaveAsRequired))
    ));
    let destination = dir.path().join("recovered.vibecut");
    a.save_as(destination.clone()).unwrap();
    saved(&mut a);
    assert!(!a.is_dirty());
    assert_eq!(
        a.saved_path(),
        Some(destination.canonicalize().unwrap().as_path())
    );
    assert_eq!(
        project::load(&destination)
            .unwrap()
            .project
            .data()
            .scene()
            .timeline
            .overlays
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
            .scene()
            .timeline
            .overlays
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
            .scene()
            .timeline
            .overlays
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
        a.switch(session(2)),
        Err(EditError::TransactionActive)
    ));
    a.session_mut().cancel_gesture().unwrap();
    a.switch(session(2)).unwrap();
    let d = discover(dir.path()).unwrap();
    assert_eq!(d.candidates.len(), 1);
    let c = d.candidates.first().unwrap();
    assert_eq!(c.journal.project_id, ProjectId(1));
    assert_eq!(
        c.recover(10)
            .unwrap()
            .project()
            .data()
            .scene()
            .timeline
            .overlays
            .len(),
        1
    );
}

#[test]
fn corrupt_journals_are_diagnosed_without_deletion_and_other_schemas_are_dropped() {
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
                json["version"] = serde_json::json!(1);
                json["project_id"] = serde_json::json!(1);
                fs::write(&meta, serde_json::to_vec(&json).unwrap()).unwrap();
            }
        }
        let before = fs::read(&meta).unwrap();
        let d = discover(dir.path()).unwrap();
        assert!(d.candidates.is_empty());
        if mode == 3 {
            assert!(d.diagnostics.is_empty());
            assert!(!meta.exists());
            assert!(
                project::autosave::list(&directory.join("project"))
                    .unwrap()
                    .is_empty()
            );
        } else {
            assert_eq!(d.diagnostics.len(), 1);
            assert_eq!(fs::read(&meta).unwrap(), before);
        }
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
            .args(["--exact", "store::recovery::recovery_child", "--nocapture"])
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
            .scene()
            .timeline
            .overlays
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
    project::WriterLease::acquire(&original)
        .unwrap()
        .save(session(1).project())
        .unwrap();
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

#[test]
fn undo_before_stale_same_path_save_keeps_current_work_recoverable() {
    stale_same_path_save_with_undo(false);
}

#[test]
fn undo_after_stale_same_path_save_keeps_current_work_recoverable() {
    stale_same_path_save_with_undo(true);
}

fn stale_same_path_save_with_undo(undo_after_completion: bool) {
    for save_current_after_flush in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("journals");
        let path = dir.path().join("saved.vibecut");
        project::WriterLease::acquire(&path)
            .unwrap()
            .save(session(1).project())
            .unwrap();
        let mut a = open(&root, 1, Some(path.clone()), Arc::new(TestClock::default()));
        edit(&mut a);
        a.save().unwrap();
        if undo_after_completion {
            edit(&mut a);
        } else {
            a.session_mut().undo().unwrap();
        }
        until(|| match a.tick() {
            Err(EditError::StaleResult) => true,
            Ok(()) => false,
            Err(e) => panic!("unexpected {e}"),
        });
        assert!(
            a.is_dirty(),
            "a stale write cannot acknowledge current work"
        );
        if undo_after_completion {
            a.session_mut().undo().unwrap();
            a.session_mut().undo().unwrap();
        }
        assert_eq!(
            a.session().project().data().scene().timeline.overlays.len(),
            0
        );
        assert_eq!(
            project::load(&path)
                .unwrap()
                .project
                .data()
                .scene()
                .timeline
                .overlays
                .len(),
            1
        );
        assert!(
            a.is_dirty(),
            "undo to the obsolete disk baseline must stay dirty"
        );
        assert_eq!(a.saved_path(), Some(path.canonicalize().unwrap().as_path()));
        a.flush().unwrap();
        if save_current_after_flush {
            a.save().unwrap();
            saved(&mut a);
            a.flush().unwrap();
            assert!(!a.is_dirty());
            assert_eq!(
                project::load(&path)
                    .unwrap()
                    .project
                    .data()
                    .scene()
                    .timeline
                    .overlays
                    .len(),
                0
            );
        }
        drop(a);
        let d = discover(&root).unwrap();
        assert!(d.diagnostics.is_empty());
        if save_current_after_flush {
            assert!(
                d.candidates.is_empty(),
                "fresh acknowledged save may retire recovery"
            );
        } else {
            assert_eq!(
                d.candidates.len(),
                1,
                "divergent work must remain recoverable"
            );
            assert_eq!(
                d.candidates
                    .first()
                    .unwrap()
                    .recover(10)
                    .unwrap()
                    .project()
                    .data()
                    .scene()
                    .timeline
                    .overlays
                    .len(),
                0
            );
        }
    }
}

fn seed(path: &Path) {
    project::WriterLease::acquire(path)
        .unwrap()
        .save(session(1).project())
        .unwrap();
}
fn busy(path: &Path) {
    assert!(matches!(
        project::WriterLease::acquire(path),
        Err(project::Error::Busy)
    ));
}
fn project_cause<'a>(error: &'a (dyn std::error::Error + 'static)) -> Option<&'a project::Error> {
    if let Some(error) = error.downcast_ref::<EditError>() {
        return match error {
            EditError::Project(error) => Some(error),
            EditError::Recovery(error) => project_cause(error),
            _ => None,
        };
    }
    match error.downcast_ref::<RecoveryError>()? {
        RecoveryError::Project(error) => Some(error),
        RecoveryError::PersistenceFailed(error) => project_cause(error.as_ref()),
        _ => None,
    }
}
#[test]
fn opened_project_holds_one_lease_before_edits_through_receipt_acknowledgement() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("opened.vibecut");
    seed(&path);
    let mut a = open(
        &dir.path().join("journals"),
        1,
        Some(path.clone()),
        Arc::new(TestClock::default()),
    );
    busy(&path);
    assert!(a.saved_receipt().is_none());
    edit(&mut a);
    busy(&path);
    a.save().unwrap();
    busy(&path);
    saved(&mut a);
    busy(&path);
    let receipt = a.saved_receipt().unwrap();
    assert_eq!(receipt.path(), path.canonicalize().unwrap());
    assert_eq!(
        receipt.content_hash(),
        media::ContentHash::from_bytes(&fs::read(&path).unwrap())
    );
    assert!(!a.is_dirty());
    drop(a);
    assert!(project::WriterLease::acquire(&path).is_ok());
}
#[test]
fn locked_invalid_project_is_refused_before_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid.vibecut");
    fs::write(&path, b"invalid json").unwrap();
    let held = project::WriterLease::acquire(&path).unwrap();
    let result = Autosave::open(
        &dir.path().join("journals"),
        &path,
        10,
        ByteBudget::new(64 * 1024 * 1024).unwrap(),
        Arc::new(TestClock::default()),
        Schedule::default(),
    );
    assert!(matches!(
        result,
        Err(EditError::Project(project::Error::Busy))
    ));
    assert!(!dir.path().join("journals").exists());
    drop(held);
    assert!(
        Autosave::open(
            &dir.path().join("journals"),
            &path,
            10,
            ByteBudget::new(64 * 1024 * 1024).unwrap(),
            Arc::new(TestClock::default()),
            Schedule::default()
        )
        .is_err()
    );
    assert!(project::WriterLease::acquire(&path).is_ok());
}
#[test]
fn foreign_edit_before_first_save_cannot_become_a_fresh_write_time_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("opened.vibecut");
    seed(&path);
    let mut a = open(
        &dir.path().join("journals"),
        1,
        Some(path.clone()),
        Arc::new(TestClock::default()),
    );
    edit(&mut a);
    let foreign = project::encode(session(2).project(), true).unwrap();
    project::atomic_write(&path, &foreign).unwrap();
    a.save().unwrap();
    until(|| match a.tick() {
        Err(error) => {
            assert!(matches!(
                project_cause(&error),
                Some(project::Error::Conflict)
            ));
            true
        }
        Ok(()) => false,
    });
    assert!(a.is_dirty());
    assert!(a.saved_receipt().is_none());
    assert_eq!(fs::read(&path).unwrap(), foreign);
    busy(&path); // Failed persistence retains the lease until the editor closes.
    assert!(a.tick().is_err());
    drop(a);
    assert!(project::WriterLease::acquire(&path).is_ok());
}
#[test]
fn stale_save_as_retains_source_lease_and_never_adopts_its_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.vibecut");
    let destination = dir.path().join("destination.vibecut");
    seed(&source);
    let original = fs::read(&source).unwrap();
    let mut a = open(
        &dir.path().join("journals"),
        1,
        Some(source.clone()),
        Arc::new(TestClock::default()),
    );
    edit(&mut a);
    a.save_as(destination.clone()).unwrap();
    busy(&source);
    busy(&destination);
    edit(&mut a);
    until(|| match a.tick() {
        Err(EditError::StaleResult) => true,
        Err(error) => panic!("unexpected {error}"),
        Ok(()) => false,
    });
    busy(&source);
    assert!(project::WriterLease::acquire(&destination).is_ok());
    assert_eq!(
        a.saved_path(),
        Some(source.canonicalize().unwrap().as_path())
    );
    assert!(a.saved_receipt().is_none());
    assert_eq!(a.session().project().data().id, ProjectId(1));
    assert!(a.is_dirty());
    assert_eq!(fs::read(&source).unwrap(), original);
    a.save().unwrap();
    saved(&mut a);
    assert!(!a.is_dirty());
    assert_eq!(
        a.saved_receipt().unwrap().path(),
        source.canonicalize().unwrap()
    );
}
#[test]
fn acknowledged_save_as_transfers_lease_and_receipt_together() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.vibecut");
    let destination = dir.path().join("destination.vibecut");
    seed(&source);
    let original = fs::read(&source).unwrap();
    let mut a = open(
        &dir.path().join("journals"),
        1,
        Some(source.clone()),
        Arc::new(TestClock::default()),
    );
    edit(&mut a);
    let old_id = a.session().project().data().id;
    edit(&mut a);
    a.session_mut().undo().unwrap();
    a.save_as(destination.clone()).unwrap();
    busy(&source);
    busy(&destination);
    saved(&mut a);
    let new_id = a.session().project().data().id;
    assert_ne!(new_id, old_id);
    assert_eq!(fs::read(&source).unwrap(), original);
    assert_eq!(
        project::load(&destination).unwrap().project.data().id,
        new_id
    );
    assert_eq!(a.saved_receipt().unwrap().project_id(), new_id);
    a.save_as(dir.path().join("./destination.vibecut")).unwrap();
    saved(&mut a);
    assert_eq!(a.session().project().data().id, new_id);
    a.session_mut().redo().unwrap();
    assert_eq!(a.session().project().data().id, new_id);
    assert_eq!(
        a.session().project().data().scene().timeline.overlays.len(),
        2
    );
    a.session_mut().undo().unwrap();
    assert!(
        fs::read_dir(dir.path().join("journals"))
            .unwrap()
            .all(|entry| !entry.unwrap().path().join("journal.json").exists())
    );
    a.session_mut().undo().unwrap();
    assert_eq!(a.session().project().data().id, new_id);
    assert_eq!(
        a.session().project().data().scene().timeline.overlays.len(),
        0
    );
    a.session_mut().redo().unwrap();
    assert_eq!(a.session().project().data().id, new_id);
    busy(&destination);
    assert!(project::WriterLease::acquire(&source).is_ok());
    assert_eq!(
        a.saved_path(),
        Some(destination.canonicalize().unwrap().as_path())
    );
    assert_eq!(a.saved_receipt().unwrap().path(), a.saved_path().unwrap());
    edit(&mut a);
    a.save().unwrap();
    saved(&mut a);
    assert_eq!(a.session().project().data().id, new_id);
    assert_eq!(
        project::load(&destination)
            .unwrap()
            .project
            .data()
            .scene()
            .timeline
            .overlays
            .len(),
        2
    );
    edit(&mut a);
    a.flush().unwrap();
    drop(a);
    let recovered = discover(&dir.path().join("journals")).unwrap();
    assert_eq!(recovered.candidates.len(), 1);
    assert_eq!(recovered.candidates[0].journal.project_id, new_id);
    assert_eq!(
        recovered.candidates[0].journal.saved_path.as_deref(),
        Some(destination.canonicalize().unwrap().as_path())
    );
    assert!(project::WriterLease::acquire(&destination).is_ok());
}
#[test]
fn save_as_refusals_preserve_source_identity_lease_and_both_files() {
    for publication_failure in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.vibecut");
        let destination = dir.path().join("destination.vibecut");
        seed(&source);
        seed(&destination);
        let source_bytes = fs::read(&source).unwrap();
        let mut destination_bytes = fs::read(&destination).unwrap();
        let held =
            (!publication_failure).then(|| project::WriterLease::acquire(&destination).unwrap());
        let budget = ByteBudget::new(64 * 1024 * 1024).unwrap();
        let pressure = budget
            .reserve(engine::ByteOwner::Compositor, 40 * 1024 * 1024)
            .unwrap();
        let root = dir.path().join("journals");
        let mut a = Autosave::open(
            &root,
            &source,
            10,
            budget,
            Arc::new(TestClock::default()),
            Schedule::default(),
        )
        .unwrap();
        edit(&mut a);
        if publication_failure {
            a.save_as(destination.clone()).unwrap();
            // The worker is queued behind the budget reservation. Change the
            // acquired destination before its final publication precondition.
            destination_bytes = project::encode(session(2).project(), true).unwrap();
            project::atomic_write(&destination, &destination_bytes).unwrap();
            drop(pressure);
            until(|| match a.tick() {
                Err(error) => {
                    assert!(matches!(
                        project_cause(&error),
                        Some(project::Error::Conflict)
                    ));
                    true
                }
                Ok(()) => false,
            });
        } else {
            let error = a.save_as(destination.clone()).unwrap_err();
            assert!(matches!(project_cause(&error), Some(project::Error::Busy)));
        }
        busy(&source);
        assert_eq!(fs::read(&source).unwrap(), source_bytes);
        assert_eq!(fs::read(&destination).unwrap(), destination_bytes);
        assert_eq!(
            a.saved_path(),
            Some(source.canonicalize().unwrap().as_path())
        );
        assert!(a.is_dirty());
        assert!(a.saved_receipt().is_none());
        assert_eq!(a.session().project().data().id, ProjectId(1));
        drop(a);
        if publication_failure {
            let recovered = discover(&root).unwrap();
            assert_eq!(recovered.candidates.len(), 1);
            assert_eq!(recovered.candidates[0].journal.project_id, ProjectId(1));
        }
        drop(held);
    }
}
#[test]
fn queued_save_cancellation_on_drop_releases_leases_without_publishing() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.vibecut");
    let destination = dir.path().join("destination.vibecut");
    seed(&source);
    let before = fs::read(&source).unwrap();
    let budget = ByteBudget::new(64 * 1024 * 1024).unwrap();
    let pressure = budget
        .reserve(engine::ByteOwner::Compositor, 40 * 1024 * 1024)
        .unwrap();
    let mut a = Autosave::open(
        &dir.path().join("journals"),
        &source,
        10,
        budget.clone(),
        Arc::new(TestClock::default()),
        Schedule::default(),
    )
    .unwrap();
    edit(&mut a);
    a.save_as(destination.clone()).unwrap();
    for _ in 0..5 {
        a.tick().unwrap();
    }
    busy(&source);
    busy(&destination);
    assert!(!destination.exists());
    assert!(a.saving());
    drop(a);
    assert_eq!(fs::read(&source).unwrap(), before);
    assert!(!destination.exists());
    assert!(project::WriterLease::acquire(&source).is_ok());
    assert!(project::WriterLease::acquire(&destination).is_ok());
    drop(pressure);
    assert_eq!(budget.snapshot().unwrap().used, 0);
}
#[cfg(unix)]
#[test]
fn save_as_alias_reuses_the_before_load_lease_and_canonical_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.vibecut");
    seed(&source);
    let alias = dir.path().join("alias.vibecut");
    std::os::unix::fs::symlink(&source, &alias).unwrap();
    let mut a = open(
        &dir.path().join("journals"),
        1,
        Some(source.clone()),
        Arc::new(TestClock::default()),
    );
    edit(&mut a);
    a.save_as(alias.clone()).unwrap();
    busy(&source);
    saved(&mut a);
    busy(&alias);
    assert!(
        fs::symlink_metadata(&alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        a.saved_receipt().unwrap().path(),
        source.canonicalize().unwrap()
    );
    assert!(!a.is_dirty());
}

#[test]
fn refused_job_submission_retains_the_before_load_lease_and_original_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.vibecut");
    seed(&source);
    let before = fs::read(&source).unwrap();
    let clock = Arc::new(TestClock::default());
    let budget = ByteBudget::new(1).unwrap();
    let mut a = Autosave::open(
        &dir.path().join("journals"),
        &source,
        10,
        budget.clone(),
        clock.clone(),
        Schedule::default(),
    )
    .unwrap();
    edit(&mut a);
    let error = a.save().unwrap_err();
    fn too_large(error: &(dyn std::error::Error + 'static)) -> bool {
        if let Some(error) = error.downcast_ref::<EditError>() {
            return match error {
                EditError::Runtime(error) => too_large(error),
                EditError::Recovery(error) => too_large(error),
                _ => false,
            };
        }
        matches!(
            error.downcast_ref::<engine::RuntimeError>(),
            Some(engine::RuntimeError::ArtifactTooLarge)
        ) || matches!(error.downcast_ref::<RecoveryError>(), Some(RecoveryError::PersistenceFailed(error)) if too_large(error.as_ref()))
    }
    assert!(too_large(&error));
    assert_eq!(error.code(), engine::control::ErrorCode::Budget);
    assert!(!error.transient());
    a.tick().unwrap(); // Explicit save admission is correctable, without a sticky cause.
    assert_eq!(budget.snapshot().unwrap().limited_used(), 0);
    busy(&source);
    assert_eq!(fs::read(&source).unwrap(), before);
    assert!(a.is_dirty());
    assert!(a.saved_receipt().is_none());
    // The same real submission refusal on the automatic path becomes sticky.
    clock.set(800);
    let error = a.tick().unwrap_err();
    assert!(too_large(&error));
    assert_eq!(error.code(), engine::control::ErrorCode::Budget);
    let EditError::Recovery(RecoveryError::PersistenceFailed(cause)) = error else {
        panic!("automatic refusal must retain its typed cause");
    };
    let EditError::Recovery(RecoveryError::PersistenceFailed(repeated)) = a.tick().unwrap_err()
    else {
        panic!("automatic refusal must remain sticky");
    };
    assert!(Arc::ptr_eq(&cause, &repeated));
    assert_eq!(budget.snapshot().unwrap().limited_used(), 0);
    busy(&source);
    assert_eq!(fs::read(&source).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn save_as_symlink_into_recovery_storage_is_refused_before_publication() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("journals");
    let mut a = open(&root, 1, None, Arc::new(TestClock::default()));
    a.flush().unwrap();
    let directory = fs::read_dir(&root).unwrap().next().unwrap().unwrap().path();
    let snapshot = project::autosave::list(&directory.join("project"))
        .unwrap()
        .pop()
        .unwrap();
    let before = fs::read(&snapshot).unwrap();
    let alias = dir.path().join("save-destination.vibecut");
    std::os::unix::fs::symlink(&snapshot, &alias).unwrap();
    let result = a.save_as(alias);
    if result.is_ok() {
        saved(&mut a);
        a.flush().unwrap();
        println!(
            "accepted protected destination; snapshot remains: {}",
            snapshot.exists()
        );
    }
    assert!(matches!(result, Err(EditError::Invalid(_))));
    assert_eq!(fs::read(&snapshot).unwrap(), before);
    assert!(a.saved_path().is_none());
    assert!(a.saved_receipt().is_none());
}

#[test]
fn save_as_admission_refusals_allow_another_destination_without_releasing_source() {
    for refuse_lock_io in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.vibecut");
        seed(&source);
        let before = fs::read(&source).unwrap();
        let mut a = open(
            &dir.path().join("journals"),
            1,
            Some(source.clone()),
            Arc::new(TestClock::default()),
        );
        edit(&mut a);
        a.flush().unwrap();
        let refused = if refuse_lock_io {
            // Existing target fits a filesystem component, but its adjacent lock
            // adds six bytes and exceeds the admitted platforms' component limit.
            let path = dir.path().join(format!("{}.vibecut", "x".repeat(242)));
            fs::write(&path, &before).unwrap();
            path
        } else {
            dir.path().join("reserved.meta.json")
        };
        let error = a.save_as(refused.clone()).unwrap_err();
        if refuse_lock_io {
            assert!(matches!(
                project_cause(&error),
                Some(project::Error::LockUnavailable(_))
            ));
        } else {
            assert!(matches!(
                project_cause(&error),
                Some(project::Error::Unsupported(_))
            ));
            assert!(!refused.exists());
        }
        a.tick().unwrap();
        assert!(!a.saving());
        assert!(a.is_dirty());
        assert_eq!(a.session().project().data().id, ProjectId(1));
        busy(&source);
        assert_eq!(fs::read(&source).unwrap(), before);
        let destination = dir.path().join("valid.vibecut");
        a.save_as(destination.clone()).unwrap();
        a.flush().unwrap();
        assert!(!a.is_dirty());
        assert_eq!(
            project::load(&destination).unwrap().project,
            **a.session().project()
        );
        assert_eq!(fs::read(&source).unwrap(), before);
        if refuse_lock_io {
            assert_eq!(fs::read(refused).unwrap(), before);
        }
    }
}

#[test]
fn clipboard_survives_refused_and_pending_save_as_and_clears_only_on_adoption() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.vibecut");
    seed(&source);
    let mut a = open(
        &dir.path().join("journals"),
        1,
        Some(source),
        Arc::new(TestClock::default()),
    );
    let mut animations = project::Animations::default();
    let mut param = project::Param::new(project::ParamValue::Scalar(0.7)).unwrap();
    param
        .channel_mut(None)
        .unwrap()
        .put(project::Keyframe::new(
            time::Tick(0),
            project::ParamValue::Scalar(0.25),
        ))
        .unwrap();
    animations
        .set(project::BuiltinPath::Opacity.into(), param)
        .unwrap();
    let outcome = a
        .session_mut()
        .apply(Command::InsertItem {
            placement: engine::Placement::Auto,
            item: engine::NewItem {
                animations,
                source_asset: None,
                source_range: time::SourceRange::new(
                    time::Tick::ZERO,
                    time::Tick(time::TICKS_PER_SECOND),
                )
                .unwrap(),
                start: time::Tick::ZERO,
                retime: Default::default(),
                content: project::ItemParams::Overlay(project::OverlayParams {
                    color: project::Color([255; 4]),
                }),
            },
        })
        .unwrap();
    let id = outcome
        .created
        .iter()
        .find_map(|id| {
            if let engine::CreatedId::Item(id) = id {
                Some(*id)
            } else {
                None
            }
        })
        .unwrap();
    a.session_mut()
        .apply(Command::CopyKeyframes {
            keys: vec![engine::KeyframeRef {
                item: id,
                path: project::BuiltinPath::Opacity.into(),
                component: None,
                time: time::Tick::ZERO,
            }],
        })
        .unwrap();
    assert!(a.save_as(dir.path().join("reserved.meta.json")).is_err());
    assert!(
        a.session_mut()
            .apply(Command::PasteKeyframes {
                item: id,
                at: time::Tick(12),
                source: engine::KeyframePasteSource::Stored
            })
            .unwrap()
            .changed
    );
    let old = a.session().project().data().id;
    a.save_as(dir.path().join("destination.vibecut")).unwrap();
    assert_eq!(a.session().project().data().id, old);
    assert!(a.session().clipboard_has_entry());
    saved(&mut a);
    assert_ne!(a.session().project().data().id, old);
    assert!(
        !a.session_mut()
            .apply(Command::PasteKeyframes {
                item: id,
                at: time::Tick(24),
                source: engine::KeyframePasteSource::Stored
            })
            .unwrap()
            .changed
    );
    let keys = a
        .session()
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == id)
        .unwrap()
        .animations
        .get(&project::BuiltinPath::Opacity.into())
        .unwrap()
        .channel(None)
        .unwrap()
        .keys();
    assert_eq!(keys.iter().map(|k| k.time.0).collect::<Vec<_>>(), [0, 12]);
}
