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
    let copied = source_writer.load().unwrap().project;
    assert_ne!(copied.data().id, project("new").data().id);
    assert_eq!(copied.data().id, receipt.project_id());
    let mut expected = project("new").into_data();
    expected.id = receipt.project_id();
    assert_eq!(copied, Project::new(expected).unwrap());
    let alias_receipt = source_writer
        .save_as(&root.path().join("./target.vibecut"), &copied)
        .unwrap();
    assert_eq!(alias_receipt.project_id(), copied.data().id);
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

#[test]
fn music_owned_directory_fresh_existing_native_paths_keep_unrelated_files() {
    use project::atomic::OwnedDirectory;
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("Âm nhạc 中");
    let root = OwnedDirectory::open(&path).unwrap();
    assert_eq!(root.path(), fs::canonicalize(&path).unwrap());
    let audio = root.child(std::ffi::OsStr::new("audio Việt")).unwrap();
    fs::write(root.path().join(".control.lock"), b"control").unwrap();
    fs::create_dir(root.path().join("unrelated")).unwrap();
    fs::write(audio.path().join("sentinel"), b"audio").unwrap();
    let reopened = OwnedDirectory::open(&path).unwrap();
    let existing = reopened.child(std::ffi::OsStr::new("audio Việt")).unwrap();
    assert_eq!(existing.path(), audio.path());
    assert_eq!(
        fs::read(existing.path().join("sentinel")).unwrap(),
        b"audio"
    );
    assert_eq!(
        fs::read(root.path().join(".control.lock")).unwrap(),
        b"control"
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 3);
    #[cfg(unix)]
    {
        use std::os::unix::{ffi::OsStringExt, fs::PermissionsExt};
        assert_eq!(
            fs::metadata(root.path()).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(audio.path()).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let name = std::ffi::OsString::from_vec(b"native-\xff".to_vec());
        match fs::create_dir(root.path().join(&name)) {
            Ok(()) => {
                let native = root.child(&name).unwrap();
                assert_eq!(native.path().file_name(), Some(name.as_os_str()));
            }
            Err(expected) => assert!(matches!(root.child(&name), Err(Error::Io(actual))
                if actual.raw_os_error() == expected.raw_os_error())),
        }
    }
    drop(root);
    audio.validate().unwrap();
    existing.validate().unwrap();
}

#[test]
fn music_owned_directory_bad_components_and_kinds_refuse_without_mutation() {
    use project::atomic::OwnedDirectory;
    use std::ffi::OsStr;
    let fixture = tempfile::tempdir().unwrap();
    let root = OwnedDirectory::open(fixture.path()).unwrap();
    fs::write(root.path().join("sentinel"), b"keep").unwrap();
    fs::write(root.path().join(".control.lock"), b"control").unwrap();
    fs::create_dir(root.path().join("unrelated")).unwrap();
    for name in ["", ".", "..", "a/b", "a\\b", "/", "a/", "./audio", "a\0b"] {
        assert!(
            matches!(root.child(OsStr::new(name)), Err(Error::Invalid(_))),
            "{name:?}"
        );
        assert_eq!(fs::read(root.path().join("sentinel")).unwrap(), b"keep");
        assert_eq!(
            fs::read(root.path().join(".control.lock")).unwrap(),
            b"control"
        );
        assert!(root.path().join("unrelated").is_dir());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 3);
    }
    #[cfg(windows)]
    for name in [
        "C:",
        "C:audio",
        "audio.",
        "audio ",
        "audio:stream",
        "NUL",
        "nul.txt",
        "CON",
        "CONIN$",
        "PRN",
        "AUX",
        "COM1",
        "LPT9.wav",
        "COM¹",
        "LPT².txt",
        "bad?",
        "bad\u{0001}",
    ] {
        assert!(
            matches!(root.child(OsStr::new(name)), Err(Error::Invalid(_))),
            "{name:?}"
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 3);
    }
    assert!(matches!(
        root.child(OsStr::new("sentinel")),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        OwnedDirectory::open(&root.path().join("sentinel")),
        Err(Error::Unsupported(_))
    ));
    let audio = root.child(OsStr::new("audio")).unwrap();
    assert!(matches!(
        audio.child(OsStr::new("nested")),
        Err(Error::Unsupported(_))
    ));
    assert!(!audio.path().join("nested").exists());
    assert_eq!(fs::read(root.path().join("sentinel")).unwrap(), b"keep");
    assert_eq!(
        fs::read(root.path().join(".control.lock")).unwrap(),
        b"control"
    );
    assert!(root.path().join("unrelated").is_dir());
}

#[cfg(unix)]
fn music_directory_alias(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}
#[cfg(windows)]
fn music_directory_alias(target: &Path, link: &Path) {
    // Test-only junction creation needs no symlink privilege. Always execute;
    // unavailable junction support is a failed fixture, never a passing skip.
    let output = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "junction fixture failed: {:?}",
        output
    );
}

#[cfg(any(unix, windows))]
#[test]
fn music_owned_directory_root_alias_resolves_once_children_refuse_aliases() {
    use project::atomic::OwnedDirectory;
    use std::ffi::OsStr;
    let fixture = tempfile::tempdir().unwrap();
    let real = fixture.path().join("real root");
    let other = fixture.path().join("unrelated");
    fs::create_dir(&real).unwrap();
    fs::create_dir(&other).unwrap();
    fs::write(real.join(".control.lock"), b"control").unwrap();
    fs::write(other.join("sentinel"), b"unrelated").unwrap();
    let alias = fixture.path().join("root alias");
    music_directory_alias(&real, &alias);
    let root = OwnedDirectory::open(&alias).unwrap();
    assert_eq!(root.path(), fs::canonicalize(&real).unwrap());
    #[cfg(windows)]
    fs::remove_dir(&alias).unwrap();
    #[cfg(unix)]
    fs::remove_file(&alias).unwrap();
    music_directory_alias(&other, &alias);
    root.validate().unwrap();
    let audio = root.child(OsStr::new("audio")).unwrap();
    assert_eq!(audio.path().parent(), Some(root.path()));
    assert!(!other.join("audio").exists());
    for (name, target) in [
        ("linked", other.clone()),
        ("dangling", fixture.path().join("absent")),
    ] {
        music_directory_alias(&target, &real.join(name));
        assert!(matches!(root.child(OsStr::new(name)), Err(Error::Conflict)));
        assert_eq!(fs::read(other.join("sentinel")).unwrap(), b"unrelated");
        assert_eq!(fs::read(real.join(".control.lock")).unwrap(), b"control");
        assert!(!fixture.path().join("absent").exists());
    }
    #[cfg(windows)]
    {
        match std::os::windows::fs::symlink_dir(&other, real.join("symlink")) {
            Ok(()) => {
                assert!(matches!(
                    root.child(OsStr::new("symlink")),
                    Err(Error::Conflict)
                ));
                std::os::windows::fs::symlink_dir(
                    fixture.path().join("absent"),
                    real.join("missing-link"),
                )
                .unwrap();
                assert!(matches!(
                    root.child(OsStr::new("missing-link")),
                    Err(Error::Conflict)
                ));
                eprintln!(
                    "Windows symlink and dangling-symlink rows executed; junction rows also executed"
                );
            }
            Err(error) if error.raw_os_error() == Some(1314) => {
                eprintln!(
                    "UNVERIFIED Windows symlink rows: ERROR_PRIVILEGE_NOT_HELD (1314); mandatory junction and dangling-junction rows executed"
                );
            }
            Err(error) => panic!("unexpected symlink fixture failure: {error}"),
        }
    }
    assert_eq!(fs::read(other.join("sentinel")).unwrap(), b"unrelated");
    assert_eq!(fs::read(real.join(".control.lock")).unwrap(), b"control");
    audio.validate().unwrap();
}

#[cfg(any(unix, windows))]
#[test]
fn music_owned_directory_retained_parent_and_child_handle_bind_replacements() {
    use project::atomic::OwnedDirectory;
    for parent in [false, true] {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("root");
        let root = OwnedDirectory::open(&path).unwrap();
        let audio = root.child(std::ffi::OsStr::new("audio")).unwrap();
        fs::write(audio.path().join("sentinel"), b"keep").unwrap();
        fs::write(root.path().join(".control.lock"), b"control").unwrap();
        let named = if parent {
            path.clone()
        } else {
            path.join("audio")
        };
        let moved = fixture.path().join("moved");
        #[cfg(windows)]
        let previous_identity = file_id::get_high_res_file_id(&named).unwrap();
        drop(root); // The child alone must retain the root handle and identity.
        let (go, ready) = std::sync::mpsc::channel();
        let attempt = std::thread::scope(|scope| {
            let named = &named;
            let moved = &moved;
            let pending = scope.spawn(move || {
                ready.recv_timeout(Duration::from_secs(10)).unwrap();
                fs::rename(named, moved)
            });
            audio.validate().unwrap();
            go.send(()).unwrap();
            pending.join().unwrap()
        });
        #[cfg(windows)]
        {
            let error = attempt.unwrap_err();
            eprintln!("Windows held directory rename refused (parent={parent}): {error:?}");
            assert!(matches!(error.raw_os_error(), Some(5 | 32)), "{error:?}");
            audio.validate().unwrap();
            assert_eq!(fs::read(audio.path().join("sentinel")).unwrap(), b"keep");
            drop(audio);
            fs::rename(&named, &moved).unwrap();
        }
        #[cfg(unix)]
        {
            attempt.unwrap();
            assert!(matches!(audio.validate(), Err(Error::Conflict)));
            drop(audio);
        }
        fs::create_dir(&named).unwrap();
        fs::write(named.join("replacement"), b"new object").unwrap();
        let reopened = OwnedDirectory::open(&named).unwrap();
        reopened.validate().unwrap();
        #[cfg(windows)]
        assert_ne!(
            file_id::get_high_res_file_id(reopened.path()).unwrap(),
            previous_identity
        );
        assert_eq!(
            fs::read(reopened.path().join("replacement")).unwrap(),
            b"new object"
        );
        let sentinel = if parent {
            moved.join("audio/sentinel")
        } else {
            moved.join("sentinel")
        };
        assert_eq!(fs::read(sentinel).unwrap(), b"keep");
        assert_eq!(
            fs::read(if parent {
                moved.join(".control.lock")
            } else {
                path.join(".control.lock")
            })
            .unwrap(),
            b"control"
        );
    }
}

#[cfg(unix)]
#[test]
fn music_owned_directory_stale_root_refuses_mutation_and_changed_child_kinds() {
    use project::atomic::OwnedDirectory;
    use std::ffi::OsStr;
    for replacement in ["absent", "file", "alias"] {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("root");
        let root = OwnedDirectory::open(&path).unwrap();
        let audio = root.child(OsStr::new("audio")).unwrap();
        fs::write(audio.path().join("sentinel"), b"keep").unwrap();
        fs::rename(audio.path(), path.join("moved")).unwrap();
        match replacement {
            "file" => fs::write(path.join("audio"), b"unrelated").unwrap(),
            "alias" => music_directory_alias(&path.join("moved"), &path.join("audio")),
            _ => {}
        }
        assert!(matches!(audio.validate(), Err(Error::Conflict)));
        assert_eq!(fs::read(path.join("moved/sentinel")).unwrap(), b"keep");
        fs::rename(&path, fixture.path().join("old-root")).unwrap();
        fs::create_dir(&path).unwrap();
        fs::write(path.join("sentinel"), b"replacement").unwrap();
        assert!(matches!(
            root.child(OsStr::new("must-not-create")),
            Err(Error::Conflict)
        ));
        assert_eq!(fs::read_dir(&path).unwrap().count(), 1);
        assert_eq!(fs::read(path.join("sentinel")).unwrap(), b"replacement");
    }
}
