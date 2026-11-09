#![allow(
    clippy::unwrap_used,
    reason = "public writer namespace regression fixtures"
)]
use project::{Error, Project, ProjectId, WriterLease, encode};
use std::{fs, path::Path};
fn project(name: &str) -> Project {
    Project::empty(ProjectId(1), name.into()).unwrap()
}
fn seed(path: &Path, name: &str) {
    fs::write(path, encode(&project(name), true).unwrap()).unwrap();
}
/// A held writer-control file is exclusively locked on Windows, so its bytes
/// cannot be read through another handle. Check that it stays empty there;
/// the calling tests check lease contention/identity. Unix reads exact bytes.
fn control_bytes(path: &Path) -> Vec<u8> {
    #[cfg(windows)]
    {
        assert_eq!(
            fs::metadata(path).unwrap().len(),
            0,
            "writer control file changed while leased"
        );
        Vec::new()
    }
    #[cfg(not(windows))]
    {
        fs::read(path).unwrap()
    }
}

#[test]
fn save_as_must_not_replace_another_live_projects_lock_file() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target.vibecut");
    let other = root.path().join("other.vibecut");
    seed(&target, "target");
    seed(&other, "other");
    let _held = WriterLease::acquire(&target).unwrap();
    let mut other_writer = WriterLease::acquire(&other).unwrap();
    let lock_path = root.path().join(".target.vibecut.lock");
    let before = control_bytes(&lock_path);
    assert!(matches!(
        other_writer.save_as(&lock_path, &project("save-as destination")),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(WriterLease::acquire(&target), Err(Error::Busy)));
    assert_eq!(control_bytes(&lock_path), before);
    assert_eq!(other_writer.load().unwrap().project, project("other"));
    assert!(!root.path().join("..target.vibecut.lock.lock").exists());
}
#[test]
fn colliding_save_as_must_not_allow_two_successful_concurrent_saves() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target.vibecut");
    let other = root.path().join("other.vibecut");
    seed(&target, "target");
    seed(&other, "other");
    let mut held = WriterLease::acquire(&target).unwrap();
    let mut other_writer = WriterLease::acquire(&other).unwrap();
    let admitted = other_writer.save_as(
        &root.path().join(".target.vibecut.lock"),
        &project("collision"),
    );
    match WriterLease::acquire(&target) {
        Ok(mut second) => {
            let barrier = std::sync::Barrier::new(2);
            let results = std::thread::scope(|scope| {
                let a = scope.spawn(|| {
                    barrier.wait();
                    held.save(&project("first"))
                });
                let b = scope.spawn(|| {
                    barrier.wait();
                    second.save(&project("second"))
                });
                (a.join().unwrap(), b.join().unwrap())
            });
            assert!(
                !(results.0.is_ok() && results.1.is_ok()),
                "both writers report committed"
            );
            panic!("second lease was admitted, even if this schedule avoided the lost edit");
        }
        Err(Error::Busy) => {
            assert!(matches!(admitted, Err(Error::Unsupported(_))));
            held.save(&project("first")).unwrap();
        }
        Err(error) => panic!("unexpected second-writer category: {error}"),
    }
    assert_eq!(project::load(&target).unwrap().project, project("first"));
    assert_eq!(other_writer.load().unwrap().project, project("other"));
}
#[test]
fn reusable_and_not_yet_existing_sidecars_are_never_project_destinations() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target.vibecut");
    seed(&target, "target");
    drop(WriterLease::acquire(&target).unwrap());
    for name in [".target.vibecut.lock", ".new.lock", ".NEW.LOCK", ".LoCk"] {
        let path = root.path().join(name);
        assert!(
            matches!(WriterLease::acquire(&path), Err(Error::Unsupported(_))),
            "reserved name admitted"
        );
    }
    assert!(!root.path().join("..new.lock.lock").exists());
    for name in ["notes.lock", ".draft.vibecut", "lock"] {
        let mut lease = WriterLease::acquire(&root.path().join(name)).unwrap();
        lease.save(&project("ordinary")).unwrap();
    }
}
#[cfg(unix)]
#[test]
fn symlink_directory_and_case_aliases_cannot_bypass_control_namespace_admission() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target.vibecut");
    let other = root.path().join("other.vibecut");
    seed(&target, "target");
    seed(&other, "other");
    let _held = WriterLease::acquire(&target).unwrap();
    let mut other_writer = WriterLease::acquire(&other).unwrap();
    let control = root.path().join(".target.vibecut.lock");
    let direct_alias = root.path().join("looks-like-project.vibecut");
    symlink(&control, &direct_alias).unwrap();
    let parent_alias = root.path().join("directory-alias");
    symlink(root.path(), &parent_alias).unwrap();
    let before = fs::read(&control).unwrap();
    for path in [
        direct_alias,
        parent_alias.join(".target.vibecut.lock"),
        root.path().join(".").join(".target.vibecut.lock"),
        root.path().join(".TARGET.VIBECUT.LOCK"),
    ] {
        assert!(matches!(
            other_writer.save_as(&path, &project("refused")),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            WriterLease::acquire(&path),
            Err(Error::Unsupported(_))
        ));
    }
    assert_eq!(fs::read(control).unwrap(), before);
    assert!(matches!(WriterLease::acquire(&target), Err(Error::Busy)));
}
#[cfg(unix)]
#[test]
fn namespace_admission_preserves_non_utf8_filename_identity() {
    use std::os::unix::ffi::OsStringExt;
    let root = tempfile::tempdir().unwrap();
    let path = root
        .path()
        .join(std::ffi::OsString::from_vec(b".native-\xff.LOCK".to_vec()));
    assert!(matches!(
        WriterLease::acquire(&path),
        Err(Error::Unsupported(_))
    ));
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}
#[test]
fn replaced_sidecar_invalidates_the_old_lease_even_when_both_files_stay_regular() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target.vibecut");
    seed(&target, "target");
    let mut old = WriterLease::acquire(&target).unwrap();
    let control = root.path().join(".target.vibecut.lock");
    let bytes = control_bytes(&control);
    let replacement = root.path().join("foreign-replacement");
    fs::write(&replacement, &bytes).unwrap();
    match fs::rename(&replacement, &control) {
        Ok(()) => {
            assert!(matches!(old.load(), Err(Error::Conflict)));
            let rescue = root.path().join("rescue.vibecut");
            assert!(matches!(
                old.save_as(&rescue, &project("stale")),
                Err(Error::Conflict)
            ));
            assert!(!rescue.exists());
            assert!(!root.path().join(".rescue.vibecut.lock").exists());
            let mut new = WriterLease::acquire(&target).unwrap();
            let barrier = std::sync::Barrier::new(2);
            let results = std::thread::scope(|scope| {
                let a = scope.spawn(|| {
                    barrier.wait();
                    old.save(&project("stale"))
                });
                let b = scope.spawn(|| {
                    barrier.wait();
                    new.save(&project("valid"))
                });
                (a.join().unwrap(), b.join().unwrap())
            });
            assert!(matches!(results.0, Err(Error::Conflict)));
            assert!(results.1.is_ok());
            assert_eq!(project::load(&target).unwrap().project, project("valid"));
        }
        Err(error) => {
            // Windows denies replacing an open sidecar when delete sharing is off.
            #[cfg(not(windows))]
            panic!("unexpected replacement refusal: {error}");
            #[cfg(windows)]
            {
                // Win32 ERROR_ACCESS_DENIED / ERROR_SHARING_VIOLATION.
                assert!(
                    error
                        .raw_os_error()
                        .is_some_and(|code| [5, 32].contains(&code))
                );
                assert!(matches!(WriterLease::acquire(&target), Err(Error::Busy)));
                old.save(&project("valid")).unwrap();
            }
        }
    }
}

#[test]
fn shared_atomic_publication_cannot_overwrite_writer_control_files() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target.vibecut");
    seed(&target, "target");
    let _held = WriterLease::acquire(&target).unwrap();
    let control = root.path().join(".target.vibecut.lock");
    let before = control_bytes(&control);
    assert!(matches!(
        project::atomic_write(&control, b"data"),
        Err(Error::Unsupported(_))
    ));
    assert_eq!(control_bytes(&control), before);
    assert!(matches!(WriterLease::acquire(&target), Err(Error::Busy)));
    #[cfg(unix)]
    {
        let alias = root.path().join("alias.vibecut");
        std::os::unix::fs::symlink(&control, &alias).unwrap();
        assert!(matches!(
            project::atomic_write(&alias, b"data"),
            Err(Error::Unsupported(_))
        ));
        assert_eq!(fs::read(&control).unwrap(), before);
    }
}

#[test]
fn every_ascii_case_spelling_of_the_control_suffix_is_reserved() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target.vibecut");
    let other = root.path().join("other.vibecut");
    seed(&target, "target");
    seed(&other, "other");
    let _held = WriterLease::acquire(&target).unwrap();
    let mut writer = WriterLease::acquire(&other).unwrap();
    for mask in 0..16 {
        let suffix: String = b"lock"
            .iter()
            .enumerate()
            .map(|(index, byte)| {
                char::from(if mask & (1 << index) == 0 {
                    *byte
                } else {
                    byte.to_ascii_uppercase()
                })
            })
            .collect();
        let destination = root.path().join(format!(".target.vibecut.{suffix}"));
        assert!(matches!(
            writer.save_as(&destination, &project("refused")),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            project::atomic_write(&destination, b"refused"),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(WriterLease::acquire(&target), Err(Error::Busy)));
    }
    assert_eq!(writer.load().unwrap().project, project("other"));
}
#[cfg(unix)]
#[test]
fn replacing_the_parent_namespace_also_invalidates_the_retained_lease() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("parent");
    fs::create_dir(&parent).unwrap();
    let target = parent.join("target.vibecut");
    seed(&target, "target");
    let mut old = WriterLease::acquire(&target).unwrap();
    fs::rename(&parent, root.path().join("moved-parent")).unwrap();
    fs::create_dir(&parent).unwrap();
    seed(&target, "target");
    let mut current = WriterLease::acquire(&target).unwrap();
    assert!(matches!(old.load(), Err(Error::Conflict)));
    assert!(matches!(old.save(&project("stale")), Err(Error::Conflict)));
    current.save(&project("current")).unwrap();
    assert_eq!(project::load(&target).unwrap().project, project("current"));
    assert_eq!(
        project::load(&root.path().join("moved-parent/target.vibecut"))
            .unwrap()
            .project,
        project("target")
    );
}
#[cfg(unix)]
#[test]
fn publishing_a_hard_link_alias_keeps_the_original_sidecar_locked() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target.vibecut");
    let other = root.path().join("other.vibecut");
    seed(&target, "target");
    seed(&other, "other");
    let mut held = WriterLease::acquire(&target).unwrap();
    let mut writer = WriterLease::acquire(&other).unwrap();
    let control = root.path().join(".target.vibecut.lock");
    let alias = root.path().join("hard-link.vibecut");
    fs::hard_link(&control, &alias).unwrap();
    let receipt = writer
        .save_as(&alias, &project("independent destination"))
        .unwrap();
    assert!(matches!(WriterLease::acquire(&target), Err(Error::Busy)));
    assert!(fs::read(&control).unwrap().is_empty());
    held.save(&project("still held")).unwrap();
    assert_eq!(
        project::load(&target).unwrap().project,
        project("still held")
    );
    let mut expected = project("independent destination").into_data();
    expected.id = receipt.project_id();
    assert_eq!(
        project::load(&alias).unwrap().project,
        Project::new(expected).unwrap()
    );
}

#[cfg(windows)]
#[test]
fn windows_normalized_and_stream_names_cannot_create_an_alias_lease() {
    let root = tempfile::tempdir().unwrap();
    for name in [
        ".new.lock.",
        ".new.lock ",
        ".new.lock::$DATA",
        "project:stream",
    ] {
        let path = root.path().join(name);
        assert!(matches!(
            WriterLease::acquire(&path),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            project::atomic_write(&path, b"data"),
            Err(Error::Unsupported(_))
        ));
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn save_to_retains_source_and_destination_leases_until_the_caller_adopts() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.vibecut");
    let destination = root.path().join("destination.vibecut");
    seed(&source, "source");
    let before = fs::read(&source).unwrap();
    let source_writer = WriterLease::acquire(&source).unwrap();
    let mut destination_writer = WriterLease::acquire(&destination).unwrap();
    let receipt = source_writer
        .save_to(&mut destination_writer, &project("copy"))
        .unwrap();
    assert_eq!(receipt.path(), destination_writer.path());
    assert_eq!(source_writer.path(), source.canonicalize().unwrap());
    assert!(matches!(WriterLease::acquire(&source), Err(Error::Busy)));
    assert!(matches!(
        WriterLease::acquire(&destination),
        Err(Error::Busy)
    ));
    assert_eq!(fs::read(&source).unwrap(), before);
    destination_writer.save(&project("next")).unwrap();
    assert_eq!(
        project::load(&destination).unwrap().project,
        project("next")
    );
}
#[cfg(unix)]
#[test]
fn invalidated_source_cannot_save_to_an_already_acquired_destination() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.vibecut");
    let destination = root.path().join("destination.vibecut");
    seed(&source, "source");
    seed(&destination, "destination");
    let source_writer = WriterLease::acquire(&source).unwrap();
    let mut destination_writer = WriterLease::acquire(&destination).unwrap();
    let replacement = root.path().join("foreign");
    fs::write(&replacement, b"").unwrap();
    fs::rename(&replacement, root.path().join(".source.vibecut.lock")).unwrap();
    let before = fs::read(&destination).unwrap();
    assert!(matches!(
        source_writer.save_to(&mut destination_writer, &project("refused")),
        Err(Error::Conflict)
    ));
    assert_eq!(fs::read(&destination).unwrap(), before);
    assert_eq!(
        destination_writer.load().unwrap().project,
        project("destination")
    );
}
