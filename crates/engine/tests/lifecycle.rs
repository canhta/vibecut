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
