#![allow(clippy::unwrap_used, reason = "synthetic project/concurrency fixtures")]
use project::{Error, Project, ProjectId, WriterLease, encode};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
fn project(name: &str) -> Project {
    Project::empty(ProjectId(1), name.into()).unwrap()
}
fn seed(path: &Path) {
    fs::write(path, encode(&project("initial"), true).unwrap()).unwrap();
}

#[test]
fn public_lease_load_save_receipt_and_atomic_replacement_keep_one_lock() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("Dự án 中.vibecut");
    seed(&path);
    let mut writer = WriterLease::acquire(&path).unwrap();
    assert_eq!(writer.load().unwrap().project, project("initial"));
    for name in ["first", "second"] {
        let receipt = writer.save(&project(name)).unwrap();
        assert_eq!(receipt.path(), writer.path());
        assert_eq!(
            receipt.content_hash(),
            media::ContentHash::read(receipt.path()).unwrap()
        );
        assert_eq!(writer.load().unwrap().project, project(name));
        assert!(matches!(WriterLease::acquire(&path), Err(Error::Busy)));
        assert!(!format!("{writer:?} {receipt:?}").contains("Dự án"));
    }
    drop(writer);
    assert!(WriterLease::acquire(&path).is_ok());
    assert!(root.path().join(".Dự án 中.vibecut.lock").is_file());
}
#[test]
fn simultaneous_thread_writers_admit_one_before_any_edit_or_save() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("p.vibecut");
    seed(&path);
    let barrier = Arc::new(Barrier::new(2));
    let outcomes = std::thread::scope(|scope| {
        let jobs = ["one", "two"].map(|name| {
            let barrier = barrier.clone();
            let path = &path;
            scope.spawn(move || {
                let acquired = WriterLease::acquire(path);
                barrier.wait();
                match acquired {
                    Ok(mut writer) => {
                        assert_eq!(writer.load().unwrap().project, project("initial"));
                        writer.save(&project(name)).unwrap();
                        true
                    }
                    Err(Error::Busy) => false,
                    Err(error) => panic!("unexpected category: {error}"),
                }
            })
        });
        jobs.map(|job| job.join().unwrap())
    });
    assert_eq!(outcomes.into_iter().filter(|saved| *saved).count(), 1);
    let saved = project::load(&path).unwrap().project;
    assert!(saved == project("one") || saved == project("two"));
}
#[test]
fn foreign_changes_creation_and_deletion_refuse_stale_publication() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("p.vibecut");
    seed(&path);
    let mut writer = WriterLease::acquire(&path).unwrap();
    writer.load().unwrap();
    let external = encode(&project("external"), true).unwrap();
    fs::write(&path, &external).unwrap();
    assert!(matches!(
        writer.save(&project("stale")),
        Err(Error::Conflict)
    ));
    assert!(matches!(writer.load(), Err(Error::Conflict)));
    assert_eq!(fs::read(&path).unwrap(), external);
    fs::remove_file(&path).unwrap();
    assert!(matches!(
        writer.save(&project("stale")),
        Err(Error::Conflict)
    ));
    assert!(!path.exists());
    drop(writer);
    let mut writer = WriterLease::acquire(&path).unwrap();
    fs::write(&path, &external).unwrap();
    assert!(matches!(writer.save(&project("new")), Err(Error::Conflict)));
    assert_eq!(fs::read(path).unwrap(), external);
}
#[test]
fn save_as_acquires_destination_and_only_switches_path_after_success() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.vibecut");
    let target = root.path().join("target.vibecut");
    seed(&source);
    seed(&target);
    let mut source_writer = WriterLease::acquire(&source).unwrap();
    let target_writer = WriterLease::acquire(&target).unwrap();
    let initial = fs::read(&source).unwrap();
    assert!(matches!(
        source_writer.save_as(&target, &project("new")),
        Err(Error::Busy)
    ));
    assert_eq!(source_writer.path(), fs::canonicalize(&source).unwrap());
    assert_eq!(fs::read(&source).unwrap(), initial);
    assert_eq!(fs::read(&target).unwrap(), initial);
    drop(target_writer);
    let receipt = source_writer.save_as(&target, &project("new")).unwrap();
    assert_eq!(source_writer.path(), fs::canonicalize(&target).unwrap());
    assert_eq!(receipt.path(), source_writer.path());
    assert_eq!(source_writer.load().unwrap().project, project("new"));
    assert_eq!(fs::read(source).unwrap(), initial);
    assert!(matches!(WriterLease::acquire(&target), Err(Error::Busy)));
}
#[test]
fn waiting_observes_the_post_publication_state_and_cancellation_writes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("p.vibecut");
    seed(&path);
    let mut held = WriterLease::acquire(&path).unwrap();
    let before = fs::read(&path).unwrap();
    assert!(matches!(
        WriterLease::acquire_wait(&path, Duration::ZERO, &|| false),
        Err(Error::Busy)
    ));
    let cancel = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(20));
            cancel.store(true, Ordering::SeqCst);
        });
        assert!(matches!(
            WriterLease::acquire_wait(&path, Duration::from_secs(2), &|| cancel
                .load(Ordering::SeqCst)),
            Err(Error::Busy)
        ));
    });
    assert_eq!(fs::read(&path).unwrap(), before);
    std::thread::scope(|scope| {
        let waiting =
            scope.spawn(|| WriterLease::acquire_wait(&path, Duration::from_secs(30), &|| false));
        std::thread::sleep(Duration::from_millis(20));
        held.save(&project("updated")).unwrap();
        drop(held);
        let lease = waiting.join().unwrap().unwrap();
        assert_eq!(lease.load().unwrap().project, project("updated"));
    });
}
#[test]
fn unusable_lock_and_oversized_wait_refuse_without_target_mutation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("p.vibecut");
    seed(&path);
    let before = fs::read(&path).unwrap();
    fs::create_dir(root.path().join(".p.vibecut.lock")).unwrap();
    assert!(matches!(
        WriterLease::acquire(&path),
        Err(Error::Unsupported(_))
    ));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(matches!(
        WriterLease::acquire_wait(&path, Duration::MAX, &|| false),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        WriterLease::acquire_wait(&path, Duration::ZERO, &|| true),
        Err(Error::Busy)
    ));
}
#[cfg(unix)]
#[test]
fn relative_symlink_case_and_non_utf8_names_follow_native_filesystem_aliases() {
    use std::os::unix::{ffi::OsStringExt, fs::symlink};
    let cwd = std::env::current_dir().unwrap();
    let root = tempfile::tempdir_in(&cwd).unwrap();
    let path = root.path().join("project.vibecut");
    seed(&path);
    let relative = path.strip_prefix(&cwd).unwrap();
    let alias = root.path().join("alias");
    symlink(&path, &alias).unwrap();
    let directory_alias = root.path().join("dir-alias");
    symlink(root.path(), &directory_alias).unwrap();
    let held = WriterLease::acquire(&path).unwrap();
    for alias in [
        relative,
        alias.as_path(),
        &directory_alias.join("project.vibecut"),
        &root.path().join(".").join("project.vibecut"),
    ] {
        assert!(matches!(WriterLease::acquire(alias), Err(Error::Busy)));
    }
    let upper = root.path().join("PROJECT.VIBECUT");
    if upper.exists() {
        assert!(matches!(WriterLease::acquire(&upper), Err(Error::Busy)));
    } else {
        eprintln!("case-insensitive alias unverified: fixture filesystem is case-sensitive");
    }
    drop(held);
    let mut via_link = WriterLease::acquire(&alias).unwrap();
    via_link.save(&project("through alias")).unwrap();
    assert!(
        fs::symlink_metadata(&alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        project::load(&path).unwrap().project,
        project("through alias")
    );
    let native = root.path().join(std::ffi::OsString::from_vec(
        b"native-\xff.vibecut".to_vec(),
    ));
    match fs::write(&native, b"") {
        Ok(()) => {
            fs::remove_file(&native).unwrap();
            let mut native_writer = WriterLease::acquire(&native).unwrap();
            native_writer.save(&project("native filename")).unwrap();
            assert_eq!(
                native_writer.load().unwrap().project,
                project("native filename")
            );
        }
        Err(expected) => {
            // APFS can reject byte sequences that another Unix filesystem admits.
            match WriterLease::acquire(&native) {
                Err(Error::LockUnavailable(actual)) | Err(Error::Io(actual)) => {
                    assert_eq!(actual.raw_os_error(), expected.raw_os_error())
                }
                _ => panic!("unsupported native name did not return its typed filesystem error"),
            }
        }
    }
}
#[cfg(unix)]
#[test]
fn symlink_lock_is_refused_without_touching_its_target() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("p.vibecut");
    seed(&path);
    let victim = root.path().join("unrelated");
    fs::write(&victim, b"keep").unwrap();
    symlink(&victim, root.path().join(".p.vibecut.lock")).unwrap();
    assert!(matches!(
        WriterLease::acquire(&path),
        Err(Error::Unsupported(_))
    ));
    assert_eq!(fs::read(victim).unwrap(), b"keep");
}

// The test executable is also a real subprocess participant, without an app/CLI
// implementation or a second persistence path in a fixture binary.
#[test]
fn lease_subprocess() {
    let Some(root) = std::env::var_os("VIBECUT_LEASE_FIXTURE") else {
        return;
    };
    let root = PathBuf::from(root);
    let role = std::env::var("VIBECUT_LEASE_ROLE").unwrap();
    let target = root.join("project.vibecut");
    let acquired = WriterLease::acquire(&target);
    if role == "loser" {
        assert!(matches!(acquired, Err(Error::Busy)));
        return;
    }
    let mut writer = acquired.unwrap();
    assert_eq!(writer.load().unwrap().project, project("initial"));
    fs::write(root.join("ready"), b"ready").unwrap();
    let start = Instant::now();
    while !root.join("release").exists() {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "fixture release timed out"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    writer.save(&project("child saved")).unwrap();
}
struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn child(root: &Path, role: &str) -> Child {
    Child(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "writer::lease_subprocess", "--nocapture"])
            .env("VIBECUT_LEASE_FIXTURE", root)
            .env("VIBECUT_LEASE_ROLE", role)
            .spawn()
            .unwrap(),
    )
}
fn ready(root: &Path) {
    let start = Instant::now();
    while !root.join("ready").exists() {
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn real_process_contention_and_abrupt_death_release_the_os_lease() {
    for kill in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("project.vibecut");
        seed(&path);
        let mut held = child(root.path(), "holder");
        ready(root.path());
        let mut refused = child(root.path(), "loser");
        assert!(refused.0.wait().unwrap().success());
        if kill {
            held.0.kill().unwrap();
            assert!(!held.0.wait().unwrap().success());
        } else {
            fs::write(root.path().join("release"), b"go").unwrap();
            assert!(held.0.wait().unwrap().success());
        }
        let writer = WriterLease::acquire(&path).unwrap();
        assert_eq!(
            writer.load().unwrap().project,
            project(if kill { "initial" } else { "child saved" })
        );
    }
}
