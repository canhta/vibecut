#![allow(clippy::unwrap_used, clippy::panic)]
use engine::{Command, CreatedId, Session};
use project::{Color, Project, ProjectId, SceneId};
use time::Tick;
fn create(s: &mut Session, name: &str) -> SceneId {
    let result = s
        .execute("scene.create", serde_json::json!({"name":name}))
        .unwrap();
    match result.created.first() {
        Some(CreatedId::Scene(id)) => *id,
        _ => panic!("scene identity"),
    }
}
#[test]
fn scenes_isolate_content_and_views_and_delete_atomically_with_history() {
    let mut s = Session::new(Project::empty(ProjectId(1), "Scenes".into()).unwrap(), 30).unwrap();
    let a = create(&mut s, "A");
    let b = create(&mut s, "B");
    s.apply(Command::RenameScene {
        scene: a,
        name: "Đoạn mở đầu".into(),
    })
    .unwrap();
    s.undo().unwrap();
    assert_eq!(s.project().data().scenes.get(a).unwrap().name.as_str(), "A");
    s.redo().unwrap();
    let initial_history = s.history_len();
    s.execute(
        "project.set_view_state",
        serde_json::json!({"zoom":2.0,"scroll":40.0,"playhead":0}),
    )
    .unwrap();
    s.execute("scene.switch", serde_json::json!({"scene":a}))
        .unwrap();
    assert_eq!(s.history_len(), initial_history);
    assert_eq!(s.metadata().view.zoom, 1.0);
    s.apply(Command::AddBookmark {
        start: Tick(5),
        duration: Tick(0),
        note: "A".into(),
        color: Color([255; 4]),
    })
    .unwrap();
    let main = s.project().data().scene().timeline.main.id;
    s.apply(Command::InsertItem {
        placement: engine::Placement::Track { track: main },
        item: engine::NewItem {
            animations: Default::default(),
            source_asset: None,
            source_range: time::SourceRange::new(Tick(0), Tick(100)).unwrap(),
            start: Tick(0),
            content: project::ItemParams::Overlay(project::OverlayParams {
                color: Color([255, 0, 0, 255]),
            }),
        },
    })
    .unwrap();
    let a_snapshot = s.project().data().scene().clone();
    s.execute(
        "project.set_view_state",
        serde_json::json!({"zoom":3.0,"scroll":80.0,"playhead":0}),
    )
    .unwrap();
    assert_eq!(s.project().duration(), Tick(100));
    s.apply(Command::SwitchScene { scene: b }).unwrap();
    assert_eq!(s.project().duration(), Tick(0));
    assert!(s.project().data().scene().bookmarks.is_empty());
    let old_item = a_snapshot.timeline.main.items.first().unwrap().id;
    assert!(
        s.apply(Command::Delete {
            items: vec![old_item]
        })
        .is_err()
    );
    s.undo().unwrap(); // remove A's item without changing the selected scene B
    assert_eq!(s.project().data().current_scene_id, b);
    s.redo().unwrap();
    assert_eq!(s.project().data().scenes.get(a).unwrap(), &a_snapshot);
    let bytes = project::encode(s.project(), false).unwrap();
    let mut reopened = Session::new(project::decode(&bytes).unwrap().project, 10).unwrap();
    reopened
        .apply(Command::AddBookmark {
            start: Tick(0),
            duration: Tick(0),
            note: "B".into(),
            color: Color([255; 4]),
        })
        .unwrap();
    for scenes in [vec![a, SceneId(0)], vec![a, SceneId(999)], vec![a, a]] {
        let before = s.project().clone();
        let history = s.history_len();
        assert!(s.apply(Command::DeleteScenes { scenes }).is_err());
        assert_eq!(s.project(), &before);
        assert_eq!(s.history_len(), history);
    }
    s.apply(Command::SwitchScene { scene: a }).unwrap();
    assert_eq!(s.metadata().view.zoom, 3.0);
    s.apply(Command::DeleteScenes { scenes: vec![a, b] })
        .unwrap();
    assert_eq!(s.project().data().current_scene_id, SceneId(0));
    assert_eq!(s.metadata().view.zoom, 2.0);
    s.undo().unwrap();
    assert_eq!(s.project().data().scenes.get(a).unwrap(), &a_snapshot);
    assert!(s.project().data().scenes.get(b).is_some());
    s.redo().unwrap();
    assert_eq!(s.project().data().scenes.iter().count(), 1);
    let bytes = project::encode(s.project(), false).unwrap();
    let mut reopened = Session::new(project::decode(&bytes).unwrap().project, 10).unwrap();
    let fresh = create(&mut reopened, "Fresh");
    assert_ne!(fresh, SceneId(0));
    let history = reopened.history_len();
    reopened.begin_gesture().unwrap();
    assert!(
        reopened
            .apply(Command::SwitchScene { scene: fresh })
            .is_err()
    );
    reopened.cancel_gesture().unwrap();
    assert_eq!(reopened.history_len(), history);
}
