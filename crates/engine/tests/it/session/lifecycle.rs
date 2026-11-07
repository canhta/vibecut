#![allow(clippy::unwrap_used, clippy::panic)]
use engine::{Command, Session};
use project::{ProjectId, ViewState};
use serde_json::json;
use time::Tick;
#[test]
fn project_creation_rename_undo_and_view_state_have_distinct_history() {
    let mut session = Session::create(
        Command::CreateProject {
            id: ProjectId(1),
            name: "New project".into(),
        },
        10,
    )
    .unwrap();
    assert_eq!(session.project().data().canvas, project::Canvas::default());
    session
        .execute("project.rename", json!({"name":"Tiếng Việt"}))
        .unwrap();
    let snapshot = session.project().clone();
    session
        .execute(
            "project.set_view_state",
            json!({"zoom":2.0,"scroll":10.0,"playhead":50}),
        )
        .unwrap();
    assert_eq!(session.project(), &snapshot);
    assert_eq!(session.history_len(), (1, 0));
    session.undo().unwrap();
    assert_eq!(session.project().data().name.as_str(), "New project");
    assert_eq!(
        session.metadata().view,
        ViewState {
            zoom: 2.0,
            scroll: 10.0,
            playhead: Tick(50)
        }
    );
    session.redo().unwrap();
    assert_eq!(session.project(), &snapshot);
    for params in [
        json!({"zoom":0,"scroll":0,"playhead":0}),
        json!({"zoom":1,"scroll":-1,"playhead":0}),
        json!({"zoom":1,"scroll":0,"playhead":-1}),
        json!({"zoom":1,"scroll":0,"playhead":0,"extra":1}),
    ] {
        assert!(session.execute("project.set_view_state", params).is_err());
    }
    assert_eq!(session.history_len(), (1, 0));
    let metadata = session.metadata().clone();
    assert!(
        session
            .apply(Command::Batch {
                commands: vec![
                    Command::RenameProject { name: "bad".into() },
                    Command::SetViewState {
                        zoom: 3.0,
                        scroll: 0.0,
                        playhead: Tick::ZERO
                    }
                ]
            })
            .is_err()
    );
    assert_eq!(session.project(), &snapshot);
    assert_eq!(session.metadata(), &metadata);
}
#[test]
fn autosave_metadata_survives_save_as_and_reopen() {
    use engine::{
        ByteBudget,
        jobs::SystemClock,
        recovery::{Autosave, Schedule},
    };
    use std::sync::Arc;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("p.vibecut");
    let open = || {
        Autosave::open(
            &root.path().join("recovery"),
            &path,
            10,
            ByteBudget::new(64 << 20).unwrap(),
            Arc::new(SystemClock::default()),
            Schedule::default(),
        )
        .unwrap()
    };
    let session = Session::create(
        Command::CreateProject {
            id: ProjectId(1),
            name: "New project".into(),
        },
        10,
    )
    .unwrap();
    let mut a = Autosave::new(
        &root.path().join("recovery"),
        session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(SystemClock::default()),
        Schedule::default(),
    )
    .unwrap();
    a.execute(
        "project.set_view_state",
        json!({"zoom":2,"scroll":50,"playhead":3}),
    )
    .unwrap();
    a.save_as(path.clone()).unwrap();
    a.flush().unwrap();
    assert!(!a.is_dirty());
    drop(a);
    let mut a = open();
    assert_eq!(a.session().metadata().view.zoom, 2.0);
    a.execute(
        "project.set_view_state",
        json!({"zoom":4,"scroll":100,"playhead":6}),
    )
    .unwrap();
    a.execute("project.save", json!({})).unwrap();
    a.flush().unwrap();
    drop(a);
    let a = open();
    assert_eq!(a.session().metadata().view.zoom, 4.0);
}

#[test]
fn thumbnails_backfill_on_open_and_refresh_at_exit_without_preview_state() {
    use engine::{
        ByteBudget,
        jobs::SystemClock,
        recovery::{Autosave, Schedule},
    };
    use std::sync::Arc;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("thumbnail.vibecut");
    let mut lease = project::WriterLease::acquire(&path).unwrap();
    lease
        .save(&project::Project::empty(ProjectId(91), "Thumbnail".into()).unwrap())
        .unwrap();
    drop(lease);
    let mut autosave = Autosave::open(
        &root.path().join("recovery"),
        &path,
        10,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(SystemClock::default()),
        Schedule::default(),
    )
    .unwrap();
    let first = autosave.session().metadata().thumbnail.clone();
    assert!(matches!(first, project::Thumbnail::Ready { .. }));
    assert_eq!(project::read_metadata(&path).thumbnail, first);
    let mut data = autosave.session().project().data().clone();
    let frame = data.canvas.fps.tick_of(1).unwrap();
    data.scene_mut().timeline.main.items.push(project::Item {
        hidden: false,
        blend_mode: Default::default(),
        retime: Default::default(),
        masks: Vec::new(),
        effects: Vec::new(),
        animations: Default::default(),
        id: project::ItemId(6),
        source_asset: None,
        start: time::Tick::ZERO,
        source_range: time::SourceRange::new(time::Tick::ZERO, frame).unwrap(),
        content: project::ItemParams::Overlay(project::OverlayParams {
            color: project::Color([0, 255, 0, 255]),
        }),
    });
    // Change through the real command path, not a preview buffer.
    let item = data.scene().timeline.main.items[0].clone();
    autosave
        .session_mut()
        .apply(Command::InsertItem {
            placement: Default::default(),
            item: engine::NewItem {
                retime: Default::default(),
                animations: item.animations,
                source_asset: item.source_asset,
                source_range: item.source_range,
                start: item.start,
                content: item.content,
            },
        })
        .unwrap();
    autosave
        .execute(
            "project.set_view_state",
            json!({"zoom":1,"scroll":0,"playhead":frame.0*10}),
        )
        .unwrap();
    autosave.execute("project.thumbnail", json!({})).unwrap();
    let thumbnail = autosave.session().metadata().thumbnail.clone();
    assert_ne!(thumbnail, first);
    assert!(
        matches!(thumbnail, project::Thumbnail::Ready { ref rgba, .. } if rgba[0] < 10 && rgba[1] > 245 && rgba[2] < 10)
    );
    assert_eq!(project::read_metadata(&path).thumbnail, thumbnail);
    // No content-save acknowledgement or undo step is created by capture.
    assert!(autosave.is_dirty());
    assert_eq!(autosave.session().history_len(), (1, 0));
}
#[test]
fn unrenderable_thumbnail_is_a_persisted_placeholder() {
    use engine::{ByteBudget, Command, library::ProjectLibrary};
    let root = tempfile::tempdir().unwrap();
    let mut library = ProjectLibrary::open(root.path(), ByteBudget::new(1).unwrap()).unwrap();
    library
        .execute(
            ProjectId(1),
            Command::CreateProject {
                id: ProjectId(1),
                name: "Small budget".into(),
            },
        )
        .unwrap();
    assert_eq!(
        library.list().unwrap()[0].thumbnail,
        project::Thumbnail::Unavailable
    );
    library
        .execute(ProjectId(1), Command::ThumbnailProject {})
        .unwrap();
    assert_eq!(
        library.list().unwrap()[0].thumbnail,
        project::Thumbnail::Unavailable
    );
}
