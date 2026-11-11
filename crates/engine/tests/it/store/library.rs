#![allow(clippy::unwrap_used, clippy::panic)]
use engine::{ByteBudget, Command, Error as EngineError, library::ProjectLibrary};
use project::{Project, ProjectId, Thumbnail, WriterLease};
use std::{cell::Cell, fs};

fn library(root: &std::path::Path) -> ProjectLibrary {
    ProjectLibrary::open(root, ByteBudget::new(128 << 20).unwrap()).unwrap()
}
fn create(library: &mut ProjectLibrary, id: u64) {
    library
        .execute(
            ProjectId(id),
            Command::CreateProject {
                id: ProjectId(id),
                name: format!("Project {id}").into(),
            },
        )
        .unwrap();
}
#[test]
fn catalog_commands_preserve_content_and_history_and_move_to_trash() {
    let root = tempfile::tempdir().unwrap();
    let mut lib = library(root.path());
    create(&mut lib, 1);
    let original = lib.list().unwrap().remove(0);
    lib.execute(ProjectId(1), Command::DuplicateProject { id: ProjectId(2) })
        .unwrap();
    let entries = lib.list().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1].short_id, "00000002");
    let source = project::load(&original.path).unwrap().project;
    let copy = project::load(&entries[1].path).unwrap().project;
    let mut data = copy.into_data();
    data.id = ProjectId(1);
    assert_eq!(
        project::encode(&Project::new(data).unwrap(), false).unwrap(),
        project::encode(&source, false).unwrap()
    );
    let writer = WriterLease::acquire(&entries[1].path).unwrap();
    let unrelated_writer = WriterLease::acquire(&entries[0].path).unwrap();
    assert!(matches!(
        lib.undo(),
        Err(EngineError::Edit(engine::EditError::Project(
            project::Error::Busy
        )))
    ));
    assert_eq!(lib.history_len(), (1, 0));
    drop(writer);
    lib.undo().unwrap();
    assert_eq!(lib.list().unwrap().len(), 1);
    lib.redo().unwrap();
    assert_eq!(lib.list().unwrap().len(), 2);
    lib.execute(
        ProjectId(2),
        Command::RenameProject {
            name: "Tiếng Việt".into(),
        },
    )
    .unwrap();
    assert_eq!(lib.list().unwrap()[1].name.as_str(), "Tiếng Việt");
    lib.undo().unwrap();
    assert_eq!(lib.list().unwrap()[1].name, original.name);
    lib.redo().unwrap();
    for name in ["", "  ", "Project 1"] {
        assert!(
            lib.execute(ProjectId(2), Command::RenameProject { name: name.into() })
                .is_err()
        );
    }
    let writer = WriterLease::acquire(&entries[1].path).unwrap();
    assert!(matches!(
        lib.execute(ProjectId(2), Command::DeleteProject {}),
        Err(EngineError::Edit(engine::EditError::Project(
            project::Error::Busy
        )))
    ));
    assert!(entries[1].path.exists());
    drop(writer);
    lib.execute(ProjectId(2), Command::DeleteProject {})
        .unwrap();
    assert!(!entries[1].path.exists());
    assert_eq!(lib.list_trash().unwrap().len(), 1);
    assert_eq!(unrelated_writer.load().unwrap().project, source);
    drop(unrelated_writer);
    assert!(matches!(
        lib.undo(),
        Err(EngineError::Session(engine::SessionError::NoHistory))
    ));
    drop(lib);
    let reopened = library(root.path());
    assert_eq!(reopened.list().unwrap().len(), 1);
    assert_eq!(reopened.list_trash().unwrap()[0].id, ProjectId(2));
}

#[test]
fn directory_move_refusals_preserve_history_and_recoverable_bytes() {
    let root = tempfile::tempdir().unwrap();
    let mut lib = library(root.path());
    create(&mut lib, 1);
    lib.execute(ProjectId(1), Command::DuplicateProject { id: ProjectId(2) })
        .unwrap();
    let path = lib.list().unwrap()[1].path.clone();
    let directory = path.parent().unwrap();
    let retained = root.path().join(".retained");
    fs::rename(directory, &retained).unwrap();
    assert!(lib.undo().is_err());
    assert_eq!(lib.history_len(), (1, 0));
    assert_eq!(
        project::load(&retained.join("project.vibecut"))
            .unwrap()
            .project
            .data()
            .id,
        ProjectId(2)
    );
    fs::rename(&retained, directory).unwrap();
    lib.undo().unwrap();
    let hidden = fs::read_dir(root.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".undo-")
        })
        .unwrap();
    let bytes = fs::read(hidden.join("project.vibecut")).unwrap();
    // Even an empty target must not be replaced (Unix rename would allow it).
    fs::create_dir(directory).unwrap();
    assert!(matches!(
        lib.redo(),
        Err(EngineError::Edit(engine::EditError::Project(
            project::Error::Conflict
        )))
    ));
    assert_eq!(lib.history_len(), (0, 1));
    assert_eq!(fs::read(hidden.join("project.vibecut")).unwrap(), bytes);
    fs::remove_dir(directory).unwrap();
    fs::rename(&hidden, &retained).unwrap();
    assert!(lib.redo().is_err());
    assert_eq!(lib.history_len(), (0, 1));
    fs::rename(&retained, &hidden).unwrap();
    lib.redo().unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(lib.history_len(), (1, 0));
}

#[test]
fn copies_media_and_rolls_back_missing_changed_cancelled_and_busy_sources() {
    let root = tempfile::tempdir().unwrap();
    let mut lib = library(root.path());
    create(&mut lib, 1);
    let path = lib.list().unwrap()[0].path.clone();
    let media = root.path().join("Tiếng Việt bytes.dat");
    fs::write(&media, [7u8; 131072]).unwrap();
    let asset = media::SourceAsset::try_from(media::AssetData {
        ephemeral: false,
        id: media::AssetId(4),
        path: media.clone(),
        content_hash: media::ContentHash::read(&media).unwrap(),
        probe: media::SourceProbe {
            format: "synthetic".into(),
            duration: None,
            streams: Vec::new(),
        },
    })
    .unwrap();
    let mut lease = WriterLease::acquire(&path).unwrap();
    let mut data = lease.load().unwrap().project.into_data();
    data.assets.push(asset);
    let source = Project::new(data).unwrap();
    lease.save(&source).unwrap();
    assert!(
        lib.execute(ProjectId(1), Command::DuplicateProject { id: ProjectId(2) })
            .is_err()
    );
    drop(lease);
    lib.execute(ProjectId(1), Command::DuplicateProject { id: ProjectId(2) })
        .unwrap();
    let entries = lib.list().unwrap();
    let duplicate = project::load(&entries[1].path).unwrap().project;
    let copied = &duplicate.data().assets[0];
    assert_ne!(copied.path(), media);
    assert_eq!(fs::read(copied.path()).unwrap(), fs::read(&media).unwrap());
    let mut data = duplicate.into_data();
    data.id = ProjectId(1);
    data.assets = source.data().assets.clone();
    assert_eq!(
        project::encode(&Project::new(data).unwrap(), false).unwrap(),
        project::encode(&source, false).unwrap()
    );
    let calls = Cell::new(0);
    assert!(matches!(
        lib.execute_cancellable(
            ProjectId(1),
            Command::DuplicateProject { id: ProjectId(3) },
            &|| {
                calls.set(calls.get() + 1);
                calls.get() == 4
            }
        ),
        Err(EngineError::Runtime(engine::RuntimeError::Cancelled))
    ));
    fs::write(&media, b"changed").unwrap();
    assert!(
        lib.execute(ProjectId(1), Command::DuplicateProject { id: ProjectId(3) })
            .is_err()
    );
    fs::remove_file(&media).unwrap();
    assert!(
        lib.execute(ProjectId(1), Command::DuplicateProject { id: ProjectId(3) })
            .is_err()
    );
    assert_eq!(lib.list().unwrap().len(), 2);
    assert!(!fs::read_dir(root.path()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".stage-")
    }));
    // Refuse undo if another editor has changed the duplicate.
    let mut lease = WriterLease::acquire(&entries[1].path).unwrap();
    let mut data = lease.load().unwrap().project.into_data();
    data.name = "Changed elsewhere".into();
    lease.save(&Project::new(data).unwrap()).unwrap();
    drop(lease);
    assert!(matches!(
        lib.undo(),
        Err(EngineError::Edit(engine::EditError::Project(
            project::Error::Conflict
        )))
    ));
    assert_eq!(lib.list().unwrap().len(), 2);
}

#[test]
fn catalog_refuses_concurrent_handles_and_symlink_entries() {
    let root = tempfile::tempdir().unwrap();
    let mut lib = library(root.path());
    create(&mut lib, 1);
    assert!(ProjectLibrary::open(root.path(), ByteBudget::new(128 << 20).unwrap()).is_err());
    #[cfg(unix)]
    {
        let other = tempfile::tempdir().unwrap();
        lib.execute(ProjectId(1), Command::DuplicateProject { id: ProjectId(2) })
            .unwrap();
        lib.undo().unwrap();
        let hidden = fs::read_dir(root.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".undo-")
            })
            .unwrap();
        let moved = other.path().join("moved");
        fs::rename(&hidden, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &hidden).unwrap();
        assert!(lib.redo().is_err());
        assert!(moved.join("project.vibecut").exists());
        fs::remove_file(&hidden).unwrap();
        fs::rename(moved, &hidden).unwrap();
        std::os::unix::fs::symlink(other.path(), root.path().join("0000000000000002")).unwrap();
        assert!(lib.list().is_err());
        assert!(
            lib.execute(ProjectId(2), Command::DeleteProject {})
                .is_err()
        );
        assert!(other.path().exists());
    }
}

#[test]
fn listing_uses_main_duration_and_active_scene_first_frame() {
    let root = tempfile::tempdir().unwrap();
    let mut lib = library(root.path());
    create(&mut lib, 1);
    let path = lib.list().unwrap()[0].path.clone();
    let mut lease = WriterLease::acquire(&path).unwrap();
    let mut session = engine::Session::new(lease.load().unwrap().project, 10).unwrap();
    session
        .apply(Command::CreateScene {
            name: "Active".into(),
        })
        .unwrap();
    let id = session
        .project()
        .data()
        .scenes
        .iter()
        .find(|s| !s.is_main)
        .unwrap()
        .id;
    session.apply(Command::SwitchScene { scene: id }).unwrap();
    let frame = session.project().data().canvas.fps.tick_of(1).unwrap();
    session
        .apply(Command::InsertItem {
            placement: Default::default(),
            item: engine::NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: None,
                source_range: time::SourceRange::new(time::Tick::ZERO, frame).unwrap(),
                start: time::Tick::ZERO,
                content: project::ItemParams::Overlay(project::OverlayParams {
                    color: project::Color([255, 0, 0, 255]),
                }),
            },
        })
        .unwrap();
    lease.save(session.project()).unwrap();
    drop(lease);
    lib.execute(ProjectId(1), Command::ThumbnailProject {})
        .unwrap();
    let entry = lib.list().unwrap().remove(0);
    assert_eq!(entry.duration, time::Tick::ZERO);
    assert!(
        matches!(entry.thumbnail, Thumbnail::Ready { ref rgba, .. } if rgba[0] > 245 && rgba[1] < 10 && rgba[2] < 10)
    );
    drop(lib);
    assert_eq!(
        library(root.path()).list().unwrap()[0].thumbnail,
        entry.thumbnail
    );
}

fn add_asset(lib: &mut ProjectLibrary, id: u64, media: &std::path::Path) -> Project {
    let path = lib.list().unwrap()[0].path.clone();
    let asset = media::SourceAsset::try_from(media::AssetData {
        ephemeral: false,
        id: media::AssetId(9),
        path: media.to_owned(),
        content_hash: media::ContentHash::read(media).unwrap(),
        probe: media::SourceProbe {
            format: "synthetic".into(),
            duration: None,
            streams: Vec::new(),
        },
    })
    .unwrap();
    let mut lease = WriterLease::acquire(&path).unwrap();
    let mut data = lease.load().unwrap().project.into_data();
    data.assets.push(asset);
    let project = Project::new(data).unwrap();
    lease.save(&project).unwrap();
    drop(lease);
    assert_eq!(project.data().id, ProjectId(id));
    project
}

#[test]
fn delete_moves_the_complete_project_to_trash_and_restore_keeps_media() {
    let root = tempfile::tempdir().unwrap();
    let mut lib = library(root.path());
    create(&mut lib, 1);
    let external = root.path().join("clip.dat");
    fs::write(&external, [5u8; 4096]).unwrap();
    add_asset(&mut lib, 1, &external);
    // A duplicate owns copied media inside its directory, like any managed
    // project whose sources were copied in.
    lib.execute(ProjectId(1), Command::DuplicateProject { id: ProjectId(2) })
        .unwrap();
    let managed = lib.list().unwrap().remove(1);
    assert_eq!(managed.id, ProjectId(2));
    let source = project::load(&managed.path).unwrap().project;
    let asset_path = source.data().assets[0].path().to_owned();
    assert_ne!(asset_path, external);
    assert!(asset_path.exists());
    lib.execute(ProjectId(2), Command::DeleteProject {})
        .unwrap();
    assert_eq!(
        lib.list().unwrap().len(),
        1,
        "only the duplicate leaves the listing"
    );
    assert!(!managed.path.exists());
    assert!(!asset_path.exists());
    let trash = lib.list_trash().unwrap();
    assert_eq!(trash.len(), 1);
    assert_eq!(trash[0].id, ProjectId(2));
    assert_eq!(trash[0].name, source.data().name);
    let trashed = trash[0].trashed_unix_ms.unwrap();
    assert_eq!(trash[0].trash_days_left(trashed), Some(30));
    assert!(
        trash[0]
            .path
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".trash-")
    );
    // The complete directory survives, with the project's own media links
    // untouched by the move.
    assert_eq!(
        project::load(&trash[0].path).unwrap().project,
        source,
        "trash keeps the exact project bytes"
    );
    lib.execute(ProjectId(2), Command::RestoreProject {})
        .unwrap();
    assert!(lib.list_trash().unwrap().is_empty());
    let restored = lib
        .list()
        .unwrap()
        .into_iter()
        .find(|entry| entry.id == ProjectId(2))
        .unwrap();
    assert_eq!(
        project::load(&restored.path).unwrap().project,
        source,
        "restore returns the same project and media links"
    );
    assert!(asset_path.exists());
    assert_eq!(fs::read(&asset_path).unwrap(), fs::read(&external).unwrap());
    // The move record leaves with the entry.
    assert!(!restored.path.parent().unwrap().join("trash.json").exists());
}

#[test]
fn purge_removes_trash_and_retention_purges_only_recorded_expired_entries() {
    let root = tempfile::tempdir().unwrap();
    let mut lib = library(root.path());
    for id in [1, 2, 3] {
        create(&mut lib, id);
        lib.execute(ProjectId(id), Command::DeleteProject {})
            .unwrap();
    }
    let trash = lib.list_trash().unwrap();
    assert_eq!(trash.len(), 3);
    // Id 1 loses its record: unknown age is never purged automatically.
    fs::remove_file(
        trash
            .iter()
            .find(|entry| entry.id == ProjectId(1))
            .unwrap()
            .path
            .parent()
            .unwrap()
            .join("trash.json"),
    )
    .unwrap();
    // Id 3 claims to have been trashed at the Unix epoch, so it is long expired.
    fs::write(
        trash
            .iter()
            .find(|entry| entry.id == ProjectId(3))
            .unwrap()
            .path
            .parent()
            .unwrap()
            .join("trash.json"),
        br#"{"unix_ms":0}"#,
    )
    .unwrap();
    // Id 2 leaves through an explicit purge, whatever its age.
    lib.execute(ProjectId(2), Command::PurgeProject {}).unwrap();
    assert_eq!(lib.list_trash().unwrap().len(), 2);
    drop(lib);
    let mut reopened = library(root.path());
    let trash = reopened.list_trash().unwrap();
    assert_eq!(
        trash.len(),
        1,
        "only the unrecorded entry survives the automatic purge"
    );
    assert_eq!(trash[0].id, ProjectId(1));
    assert_eq!(trash[0].trashed_unix_ms, None);
    assert_eq!(trash[0].trash_days_left(0), None);
    assert!(!root.path().join(".trash-0000000000000002").exists());
    assert!(!root.path().join(".trash-0000000000000003").exists());
    assert!(root.path().join(".trash-0000000000000001").exists());
    // Purging an entry that is no longer in Trash refuses without touching the catalog.
    assert!(
        reopened
            .execute(ProjectId(2), Command::PurgeProject {})
            .is_err()
    );
    assert_eq!(reopened.list_trash().unwrap().len(), 1);
    reopened
        .execute(ProjectId(1), Command::RestoreProject {})
        .unwrap();
    assert_eq!(reopened.list().unwrap().len(), 1);
}

#[test]
fn restore_refuses_an_occupied_target_and_keeps_the_trashed_bytes() {
    let root = tempfile::tempdir().unwrap();
    let mut lib = library(root.path());
    create(&mut lib, 1);
    lib.execute(ProjectId(1), Command::DeleteProject {})
        .unwrap();
    let trashed = lib.list_trash().unwrap().remove(0);
    // A later project reuses the identity while the original sits in Trash.
    create(&mut lib, 1);
    assert!(matches!(
        lib.execute(ProjectId(1), Command::RestoreProject {}),
        Err(EngineError::Edit(engine::EditError::Project(
            project::Error::Conflict
        )))
    ));
    assert_eq!(lib.list().unwrap().len(), 1);
    assert_eq!(lib.list_trash().unwrap().len(), 1);
    assert_eq!(
        project::load(&trashed.path).unwrap().project.data().id,
        ProjectId(1)
    );
}

#[test]
fn delete_refuses_a_busy_project_and_a_hostile_trash_destination() {
    let root = tempfile::tempdir().unwrap();
    let mut lib = library(root.path());
    create(&mut lib, 1);
    let entry = lib.list().unwrap().remove(0);
    let writer = WriterLease::acquire(&entry.path).unwrap();
    assert!(matches!(
        lib.execute(ProjectId(1), Command::DeleteProject {}),
        Err(EngineError::Edit(engine::EditError::Project(
            project::Error::Busy
        )))
    ));
    assert!(entry.path.exists());
    drop(writer);
    // An occupied Trash destination refuses the move after the record was
    // staged, so the entry stays complete and active.
    let occupied = root.path().join(".trash-0000000000000001");
    fs::create_dir(&occupied).unwrap();
    assert!(
        lib.execute(ProjectId(1), Command::DeleteProject {})
            .is_err()
    );
    assert_eq!(lib.list().unwrap().len(), 1);
    // An empty `.trash-<id>` directory is a corrupt catalog entry: it is
    // reported, never silently hidden, and never replaces the active project.
    assert!(lib.list_trash().is_err());
    // The record staged before the move stays with the active entry and is
    // replaced by the next attempt; it never marks an active project trashed.
    assert!(entry.path.parent().unwrap().join("trash.json").exists());
    fs::remove_dir(&occupied).unwrap();
    assert!(lib.list_trash().unwrap().is_empty());
    lib.execute(ProjectId(1), Command::DeleteProject {})
        .unwrap();
    assert_eq!(lib.list_trash().unwrap().len(), 1);
}

#[test]
fn corrupt_trashed_entries_are_reported_and_never_hidden() {
    let root = tempfile::tempdir().unwrap();
    let mut lib = library(root.path());
    create(&mut lib, 1);
    lib.execute(ProjectId(1), Command::DeleteProject {})
        .unwrap();
    let corrupt = root.path().join(".trash-0000000000000002");
    fs::create_dir(&corrupt).unwrap();
    fs::write(corrupt.join("project.vibecut"), b"not a project").unwrap();
    assert!(lib.list_trash().is_err());
    assert_eq!(lib.list().unwrap().len(), 0, "active listing stays usable");
    assert!(lib.execute(ProjectId(2), Command::PurgeProject {}).is_err());
    fs::remove_dir_all(&corrupt).unwrap();
    assert_eq!(lib.list_trash().unwrap().len(), 1);
    // Retention boundaries, independent of the filesystem clock.
    let entry = lib.list_trash().unwrap().remove(0);
    let start = entry.trashed_unix_ms.unwrap();
    assert_eq!(entry.trash_days_left(start), Some(30));
    assert_eq!(
        entry.trash_days_left(start + 24 * 60 * 60 * 1000 - 1),
        Some(30)
    );
    assert_eq!(entry.trash_days_left(start + 24 * 60 * 60 * 1000), Some(29));
    assert_eq!(
        entry.trash_days_left(start + 29 * 24 * 60 * 60 * 1000),
        Some(1)
    );
    assert_eq!(
        entry.trash_days_left(start + 30 * 24 * 60 * 60 * 1000),
        Some(0)
    );
    assert_eq!(
        entry.trash_days_left(start + 99 * 24 * 60 * 60 * 1000),
        Some(0)
    );
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn trash_dir(root: &std::path::Path, id: u64) -> std::path::PathBuf {
    root.join(format!(".trash-{id:016x}"))
}

fn write_record(directory: &std::path::Path, unix_ms: u64) {
    fs::write(
        directory.join("trash.json"),
        format!("{{\"unix_ms\":{unix_ms}}}"),
    )
    .unwrap();
}

#[test]
fn expiry_admits_only_an_identifiable_unleased_entry() {
    let root = tempfile::tempdir().unwrap();
    let mut lib = library(root.path());
    for id in [1, 2, 3, 4] {
        create(&mut lib, id);
        lib.execute(ProjectId(id), Command::DeleteProject {})
            .unwrap();
        write_record(&trash_dir(root.path(), id), 0);
    }
    // Corrupt id 2's project and give id 3 another project's identity.
    fs::write(
        trash_dir(root.path(), 2).join("project.vibecut"),
        b"not a project",
    )
    .unwrap();
    let identity = fs::read(trash_dir(root.path(), 1).join("project.vibecut")).unwrap();
    fs::write(trash_dir(root.path(), 3).join("project.vibecut"), &identity).unwrap();
    // A retained project writer lease keeps id 1 complete across the sweep.
    let held = WriterLease::acquire(&trash_dir(root.path(), 1).join("project.vibecut")).unwrap();
    drop(lib);
    let reopened = library(root.path());
    assert!(
        !trash_dir(root.path(), 4).exists(),
        "a validated, unleased expired entry is purged"
    );
    assert!(trash_dir(root.path(), 1).exists(), "a busy entry stays");
    assert!(trash_dir(root.path(), 2).exists(), "a corrupt entry stays");
    assert!(
        trash_dir(root.path(), 3).exists(),
        "a mismatched identity stays"
    );
    assert_eq!(
        fs::read(trash_dir(root.path(), 1).join("project.vibecut")).unwrap(),
        identity,
        "the sweep never touches the held project bytes"
    );
    // The corrupt and mismatched entries are reported, never hidden; the held
    // entry is retried once its writer lease is gone.
    assert!(reopened.list_trash().is_err());
    drop(held);
    drop(reopened);
    fs::remove_dir_all(trash_dir(root.path(), 2)).unwrap();
    fs::remove_dir_all(trash_dir(root.path(), 3)).unwrap();
    let reopened = library(root.path());
    assert!(
        !trash_dir(root.path(), 1).exists(),
        "the freed entry expires on the next open"
    );
    assert!(reopened.list_trash().unwrap().is_empty());
}

#[test]
fn expiry_needs_a_recorded_age_and_a_valid_clock() {
    let root = tempfile::tempdir().unwrap();
    let mut lib = library(root.path());
    for id in [1, 2, 3, 4] {
        create(&mut lib, id);
        lib.execute(ProjectId(id), Command::DeleteProject {})
            .unwrap();
    }
    let now = now_unix_ms();
    let retention = 30 * 24 * 60 * 60 * 1000;
    write_record(&trash_dir(root.path(), 1), now - retention + 60_000);
    write_record(&trash_dir(root.path(), 2), now + 24 * 60 * 60 * 1000);
    fs::write(
        trash_dir(root.path(), 3).join("trash.json"),
        vec![b'x'; 256],
    )
    .unwrap();
    write_record(&trash_dir(root.path(), 4), now - retention);
    drop(lib);
    let reopened = library(root.path());
    let ids: Vec<ProjectId> = reopened
        .list_trash()
        .unwrap()
        .into_iter()
        .map(|entry| entry.id)
        .collect();
    assert_eq!(ids, vec![ProjectId(1), ProjectId(2), ProjectId(3)]);
    assert!(
        !trash_dir(root.path(), 4).exists(),
        "exactly 30 recorded days expires"
    );
    assert!(trash_dir(root.path(), 1).exists(), "one minute short stays");
    assert!(trash_dir(root.path(), 2).exists(), "a future record stays");
    assert!(
        trash_dir(root.path(), 3).exists(),
        "an unreadable record stays untimed"
    );
}

#[test]
fn expired_entry_moves_to_an_unlisted_tombstone_before_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let mut lib = library(root.path());
    create(&mut lib, 1);
    lib.execute(ProjectId(1), Command::DeleteProject {})
        .unwrap();
    let trashed = trash_dir(root.path(), 1);
    write_record(&trashed, 0);
    let held_path = trashed.join("held.dat");
    fs::write(&held_path, b"open").unwrap();
    // A read handle can block the recursive cleanup on Windows. The atomic
    // tombstone move already happened, so a partial cleanup can only leave an
    // unlisted `.delete-*` directory, never a visible or restorable entry.
    let held = fs::File::open(&held_path).unwrap();
    drop(lib);
    let reopened = library(root.path());
    if trashed.exists() {
        // The open handle refused the move: the entry is still complete and
        // listed at zero days, never partly destroyed.
        let entry = reopened.list_trash().unwrap().remove(0);
        assert_eq!(entry.id, ProjectId(1));
        assert_eq!(
            project::load(&entry.path).unwrap().project.data().id,
            ProjectId(1)
        );
    } else {
        // The move admitted the entry; any failed cleanup is an unlisted tombstone.
        assert!(reopened.list_trash().unwrap().is_empty());
        assert!(reopened.list().unwrap().is_empty());
    }
    drop(held);
    drop(reopened);
    // Releasing the handle lets the next open finish the destructible entry.
    let reopened = library(root.path());
    assert!(reopened.list_trash().unwrap().is_empty());
    assert!(!trashed.exists());
}

#[test]
fn hard_linked_trash_record_never_truncates_project_bytes() {
    let root = tempfile::tempdir().unwrap();
    let mut lib = library(root.path());
    create(&mut lib, 1);
    let directory = root.path().join("0000000000000001");
    let project = fs::read(directory.join("project.vibecut")).unwrap();
    fs::hard_link(
        directory.join("project.vibecut"),
        directory.join("trash.json"),
    )
    .unwrap();
    lib.execute(ProjectId(1), Command::DeleteProject {})
        .unwrap();
    assert_eq!(
        fs::read(trash_dir(root.path(), 1).join("project.vibecut")).unwrap(),
        project,
        "publishing the record replaces the link, never the project inode"
    );
    lib.execute(ProjectId(1), Command::RestoreProject {})
        .unwrap();
    assert_eq!(
        fs::read(directory.join("project.vibecut")).unwrap(),
        project
    );
}

#[test]
fn hard_linked_trash_record_never_truncates_media_bytes() {
    let root = tempfile::tempdir().unwrap();
    let mut lib = library(root.path());
    create(&mut lib, 1);
    let external = root.path().join("clip.dat");
    fs::write(&external, [7u8; 4096]).unwrap();
    add_asset(&mut lib, 1, &external);
    lib.execute(ProjectId(1), Command::DuplicateProject { id: ProjectId(2) })
        .unwrap();
    let project = project::load(&lib.list().unwrap()[1].path).unwrap().project;
    let media = project.data().assets[0].path().to_owned();
    let bytes = fs::read(&media).unwrap();
    let directory = root.path().join("0000000000000002");
    fs::hard_link(&media, directory.join("trash.json")).unwrap();
    lib.execute(ProjectId(2), Command::DeleteProject {})
        .unwrap();
    assert_eq!(
        fs::read(
            trash_dir(root.path(), 2)
                .join("media")
                .join(media.file_name().unwrap())
        )
        .unwrap(),
        bytes,
        "publishing the record never truncates the media inode"
    );
    lib.execute(ProjectId(2), Command::RestoreProject {})
        .unwrap();
    assert_eq!(fs::read(&media).unwrap(), bytes);
    assert_eq!(
        project::load(&lib.list().unwrap()[1].path).unwrap().project,
        project
    );
}
