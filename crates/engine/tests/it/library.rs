#![allow(clippy::unwrap_used, clippy::panic)]
use engine::{ByteBudget, Command, EditError, library::ProjectLibrary};
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
fn catalog_commands_preserve_content_and_history_and_permanent_delete() {
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
    lib.execute(ProjectId(2), Command::DeleteProject {})
        .unwrap();
    assert!(!entries[1].path.exists());
    assert!(matches!(lib.undo(), Err(EditError::NoHistory)));
    drop(lib);
    assert_eq!(library(root.path()).list().unwrap().len(), 1);
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
        Err(EditError::Cancelled)
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
        Err(EditError::Project(project::Error::Conflict))
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
        .execute("scene.create", serde_json::json!({"name":"Active"}))
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
