#![allow(clippy::unwrap_used, clippy::panic)]
use engine::{Command, Session};
use project::*;
use serde_json::json;
use time::{FrameRate, SourceRange, Tick};
fn session() -> Session {
    let mut data = Project::empty(ProjectId(1), "Effects".into())
        .unwrap()
        .into_data();
    data.timeline.tracks.push(Track {
        id: TrackId(1),
        name: "V".into(),
        kind: TrackKind::Overlay,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            effects: vec![],
            animations: Default::default(),
            id: ItemId(2),
            source_asset: None,
            source_range: SourceRange::new(Tick::ZERO, FrameRate::FPS_30.tick_of(30).unwrap())
                .unwrap(),
            start: Tick::ZERO,
            content: ItemParams::Overlay(OverlayParams {
                color: Color([255; 4]),
            }),
        }],
    });
    Session::new(Project::new(data).unwrap(), 20).unwrap()
}
#[test]
fn registry_effect_edits_persist_and_undo_atomically() {
    let mut s = session();
    let before = s.project().clone();
    s.execute("effects.add", json!({"item":2,"effect":{"id":"blur1","enabled":true,"params":{"kind":"blur","intensity":15.0}}})).unwrap();
    let added = s.project().clone();
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &before);
    s.apply(Command::Redo {}).unwrap();
    assert_eq!(s.project(), &added);
    for bad in [-1.0, 101.0] {
        assert!(
            s.execute(
                "effects.update_params",
                json!({"item":2,"effect":"blur1","params":{"kind":"blur","intensity":bad}})
            )
            .is_err()
        );
        assert_eq!(s.project(), &added);
    }
    assert!(s.execute("effects.add", json!({"item":2,"effect":{"id":"blur1","enabled":true,"params":{"kind":"blur","intensity":15.0}}})).is_err());
    s.execute(
        "effects.update_params",
        json!({"item":2,"effect":"blur1","params":{"kind":"blur","intensity":35.0}}),
    )
    .unwrap();
    let path = AnimationPath::new("effects.blur1.params.intensity").unwrap();
    s.apply(Command::SetAnimatedParam {
        item: ItemId(2),
        path: path.clone(),
        param: Param::new(ParamValue::Scalar(70.0)).unwrap(),
    })
    .unwrap();
    let saved = project::encode(s.project(), false).unwrap();
    assert_eq!(
        &project::decode(&saved).unwrap().project,
        s.project().as_ref()
    );
    s.execute("effects.remove", json!({"item":2,"effect":"blur1"}))
        .unwrap();
    let item = s
        .project()
        .data()
        .timeline
        .tracks
        .first()
        .unwrap()
        .items
        .first()
        .unwrap();
    assert!(item.effects.is_empty());
    assert!(item.animations.get(&path).is_none());
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), saved);
    let old = s.project().clone();
    assert!(
        s.execute("effects.remove", json!({"item":2,"effect":"missing"}))
            .is_err()
    );
    assert_eq!(s.project(), &old);
    s.apply(Command::SetTrackFlags {
        track: TrackId(1),
        muted: false,
        hidden: false,
        locked: true,
    })
    .unwrap();
    assert!(
        s.execute("effects.remove", json!({"item":2,"effect":"blur1"}))
            .is_err()
    );
}
