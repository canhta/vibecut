#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
use engine::{CreatedId, Session};
use project::{Project, ProjectId, TrackKind};
use serde_json::json;
fn session() -> Session {
    Session::new(Project::empty(ProjectId(1), "Tracks".into()).unwrap(), 100).unwrap()
}
#[test]
fn commands_protect_main_and_restore_items_flags_and_magnet() {
    let mut s = session();
    let initial = s.project().clone();
    assert!(s.execute("track.remove", json!({"track":0})).is_err());
    assert_eq!(s.project(), &initial);
    let result = s
        .execute("track.add", json!({"name":"Video overlay","kind":"video"}))
        .unwrap();
    let CreatedId::Track(id) = result.created[0] else {
        panic!("track")
    };
    s.execute("insert_item",json!({"track":id,"item":{"animations":{},"source_asset":null,"source_range":{"start":0,"end":100},"start":0,"content":{"kind":"overlay","params":{"color":[0,0,255,255]}}}})).unwrap();
    let populated = s.project().clone();
    s.execute("track.set_muted", json!({"track":id,"muted":true}))
        .unwrap();
    s.execute("track.set_hidden", json!({"track":id,"hidden":true}))
        .unwrap();
    let flagged = s.project().clone();
    assert!(flagged.data().timeline.overlays[0].muted);
    assert!(flagged.data().timeline.overlays[0].hidden);
    s.execute("track.remove", json!({"track":id})).unwrap();
    assert!(s.project().data().timeline.overlays.is_empty());
    s.execute("undo", json!({})).unwrap();
    assert_eq!(s.project(), &flagged);
    s.execute("undo", json!({})).unwrap();
    s.execute("undo", json!({})).unwrap();
    assert_eq!(s.project(), &populated);
    s.execute("redo", json!({})).unwrap();
    s.execute("redo", json!({})).unwrap();
    assert_eq!(s.project(), &flagged);
    s.execute("track.set_magnet", json!({"enabled":false}))
        .unwrap();
    assert!(!s.project().data().timeline.magnet);
    s.execute("undo", json!({})).unwrap();
    assert!(s.project().data().timeline.magnet);
    s.execute("redo", json!({})).unwrap();
    assert!(!s.project().data().timeline.magnet);
}
#[test]
fn kinds_controls_refuse_wrong_targets_atomically_and_have_one_registry_name() {
    let mut s = session();
    for kind in [
        TrackKind::Text,
        TrackKind::Sticker,
        TrackKind::Effect,
        TrackKind::Filter,
        TrackKind::Adjustment,
        TrackKind::Audio,
    ] {
        let result = s
            .execute("track.add", json!({"name":"Track","kind":kind}))
            .unwrap();
        let CreatedId::Track(id) = result.created[0] else {
            panic!("track")
        };
        let before = s.project().clone();
        let (cmd, params) = if kind == TrackKind::Audio {
            ("track.set_hidden", json!({"track":id,"hidden":true}))
        } else {
            ("track.set_muted", json!({"track":id,"muted":true}))
        };
        assert!(s.execute(cmd, params).is_err());
        assert_eq!(s.project(), &before);
        let (cmd, params) = if kind == TrackKind::Audio {
            ("track.set_muted", json!({"track":id,"muted":true}))
        } else {
            ("track.set_hidden", json!({"track":id,"hidden":true}))
        };
        s.execute(cmd, params).unwrap();
        let changed = s
            .project()
            .data()
            .timeline
            .iter()
            .find(|track| track.id == id)
            .unwrap();
        assert!(if kind == TrackKind::Audio {
            changed.muted
        } else {
            changed.hidden
        });
        s.execute("track.remove", json!({"track":id})).unwrap();
        assert!(
            !s.project()
                .data()
                .timeline
                .iter()
                .any(|track| track.id == id)
        );
    }
    for old in ["add_track", "remove_track", "set_track_flags"] {
        assert!(!engine::registry().iter().any(|s| s.id == old));
    }
    assert!(
        s.execute("track.set_hidden", json!({"track":u64::MAX,"hidden":true}))
            .is_err()
    );
}
