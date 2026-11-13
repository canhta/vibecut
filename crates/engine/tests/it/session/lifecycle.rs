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
            fps: 30,
        },
        10,
    )
    .unwrap();
    assert_eq!(session.project().data().canvas, project::Canvas::default());
    // The creation frame rate is the preference the caller passes; 60 is a listed choice.
    let fast = Session::create(
        Command::CreateProject {
            id: ProjectId(2),
            name: "Fast".into(),
            fps: 60,
        },
        10,
    )
    .unwrap();
    assert_eq!(
        fast.project().data().canvas.fps,
        time::FrameRate::new(60, 1).unwrap()
    );
    session
        .apply(Command::RenameProject {
            name: "Tiếng Việt".into(),
        })
        .unwrap();
    let snapshot = session.project().clone();
    session
        .apply(Command::SetViewState {
            zoom: 2.0,
            scroll: 10.0,
            playhead: Tick(50),
        })
        .unwrap();
    assert_eq!(session.project(), &snapshot);
    assert_eq!(session.history_len(), (1, 0));
    // The proxy choice is presentation state beside the view: no history entry
    // and no change to the project snapshot or its serialized bytes.
    let bytes = serde_json::to_vec(session.project().data()).unwrap();
    assert!(!session.metadata().proxy_preview);
    assert!(session.set_proxy_preview(true).unwrap());
    assert!(!session.set_proxy_preview(true).unwrap());
    assert!(session.metadata().proxy_preview);
    assert_eq!(session.project(), &snapshot);
    assert_eq!(session.history_len(), (1, 0));
    assert_eq!(serde_json::to_vec(session.project().data()).unwrap(), bytes);
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
    for view in [
        ViewState {
            zoom: 0.0,
            scroll: 0.0,
            playhead: Tick(0),
        },
        ViewState {
            zoom: 1.0,
            scroll: -1.0,
            playhead: Tick(0),
        },
        ViewState {
            zoom: 1.0,
            scroll: 0.0,
            playhead: Tick(-1),
        },
    ] {
        assert!(
            session
                .apply(Command::SetViewState {
                    zoom: view.zoom,
                    scroll: view.scroll,
                    playhead: view.playhead,
                })
                .is_err()
        );
    }
    assert!(
        crate::wire::run(
            &mut session,
            "project.set_view_state",
            json!({"zoom":1,"scroll":0,"playhead":0,"extra":1})
        )
        .is_err()
    );
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
fn autosave_metadata_survives_save_to_and_reopen() {
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
            fps: 30,
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
    a.apply(Command::SetViewState {
        zoom: 2.0,
        scroll: 50.0,
        playhead: Tick(3),
    })
    .unwrap();
    a.save_to(path.clone()).unwrap();
    a.flush().unwrap();
    assert!(!a.is_dirty());
    drop(a);
    let mut a = open();
    assert_eq!(a.session().metadata().view.zoom, 2.0);
    a.apply(Command::SetViewState {
        zoom: 4.0,
        scroll: 100.0,
        playhead: Tick(6),
    })
    .unwrap();
    a.apply(Command::SaveProject {}).unwrap();
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
        .apply(Command::SetViewState {
            zoom: 1.0,
            scroll: 0.0,
            playhead: Tick(frame.0 * 10),
        })
        .unwrap();
    autosave.apply(Command::ThumbnailProject {}).unwrap();
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
                fps: 30,
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
