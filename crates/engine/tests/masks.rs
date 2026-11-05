#![allow(clippy::unwrap_used, clippy::panic)]
use engine::{Command, Session};
use project::*;
use serde_json::json;
use time::{FrameRate, SourceRange, Tick};
fn session() -> Session {
    let mut data = Project::empty(ProjectId(1), "Masks".into())
        .unwrap()
        .into_data();
    data.timeline.insert_track(Track {
        id: TrackId(1),
        name: "V".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            masks: Vec::new(),
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
fn mask_commands_validate_persist_order_and_restore_history_atomically() {
    let mut s = session();
    let start = s.project().clone();
    let mut a = Mask::new("a", MaskShape::Rectangle);
    a.feather = 12.;
    a.opacity = 0.5;
    a.stroke.width = 2.;
    a.stroke.align = StrokeAlign::Outside;
    s.execute("masks.add", json!({"item":2,"mask":a})).unwrap();
    let added = s.project().clone();
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &start);
    s.apply(Command::Redo {}).unwrap();
    assert_eq!(s.project(), &added);
    for n in 0..12 {
        s.apply(Command::AddMask {
            item: ItemId(2),
            mask: Mask::new(format!("m{n}"), MaskShape::Ellipse),
        })
        .unwrap();
    }
    let mut order: Vec<_> = s
        .project()
        .data()
        .timeline
        .overlays
        .first()
        .unwrap()
        .items
        .first()
        .unwrap()
        .masks
        .iter()
        .map(|m| m.id.clone())
        .collect();
    order.reverse();
    s.execute("masks.reorder", json!({"item":2,"order":order}))
        .unwrap();
    let reordered = s.project().clone();
    let mut replacement = a.clone();
    replacement.shape = MaskShape::Star;
    replacement.inverted = true;
    s.execute("masks.update", json!({"item":2,"mask":replacement}))
        .unwrap();
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &reordered);
    s.apply(Command::Redo {}).unwrap();
    let path = a.path("feather").unwrap();
    s.apply(Command::SetAnimatedParam {
        item: ItemId(2),
        path: path.clone(),
        param: Param::new(ParamValue::Scalar(5.)).unwrap(),
    })
    .unwrap();
    let saved = project::encode(s.project(), false).unwrap();
    assert_eq!(
        &project::decode(&saved).unwrap().project,
        s.project().as_ref()
    );
    s.execute("masks.remove", json!({"item":2,"mask":"a"}))
        .unwrap();
    assert!(
        s.project()
            .data()
            .timeline
            .overlays
            .first()
            .unwrap()
            .items
            .first()
            .unwrap()
            .animations
            .get(&path)
            .is_none()
    );
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), saved);
    for request in [
        json!({"item":2,"mask":a}),
        json!({"item":2,"mask":"missing"}),
    ] {
        let command = if request.get("mask").unwrap().is_string() {
            "masks.remove"
        } else {
            "masks.add"
        };
        let before = s.project().clone();
        assert!(s.execute(command, request).is_err());
        assert_eq!(s.project(), &before);
    }
    for order in [vec!["a".to_owned()], vec!["a".to_owned(); 13]] {
        let before = s.project().clone();
        assert!(
            s.execute("masks.reorder", json!({"item":2,"order":order}))
                .is_err()
        );
        assert_eq!(s.project(), &before);
    }
    for value in [f64::NAN, -1., 10001.] {
        let mut bad = Mask::new("bad", MaskShape::Rectangle);
        bad.feather = value;
        let before = s.project().clone();
        assert!(
            s.apply(Command::AddMask {
                item: ItemId(2),
                mask: bad
            })
            .is_err()
        );
        assert_eq!(s.project(), &before);
    }
    let mut bad = Mask::new(
        "bad",
        MaskShape::Path {
            path: MaskPath {
                closed: true,
                vertices: vec![MaskVertex {
                    p: [f64::INFINITY, 0.],
                    t_in: [0.; 2],
                    t_out: [0.; 2],
                }],
            },
        },
    );
    assert!(
        s.apply(Command::AddMask {
            item: ItemId(2),
            mask: bad.clone()
        })
        .is_err()
    );
    bad.id = "invalid.id".into();
    assert!(
        s.apply(Command::AddMask {
            item: ItemId(2),
            mask: bad
        })
        .is_err()
    );
    s.apply(Command::SetTrackFlags {
        track: TrackId(1),
        muted: false,
        hidden: false,
        locked: true,
    })
    .unwrap();
    let before = s.project().clone();
    assert!(
        s.execute("masks.remove", json!({"item":2,"mask":"a"}))
            .is_err()
    );
    assert_eq!(s.project(), &before);
}
