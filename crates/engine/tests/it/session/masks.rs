#![allow(clippy::unwrap_used, clippy::panic)]
use engine::{Command, Session};
use project::*;
use time::{FrameRate, SourceRange, Tick};
fn session() -> Session {
    let mut data = Project::empty(ProjectId(1), "Masks".into())
        .unwrap()
        .into_data();
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(1),
        name: "V".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
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
    s.apply(Command::AddMask {
        item: ItemId(2),
        mask: a.clone(),
    })
    .unwrap();
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
        .scene()
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
    s.apply(Command::ReorderMasks {
        item: ItemId(2),
        order,
    })
    .unwrap();
    let reordered = s.project().clone();
    let mut replacement = a.clone();
    replacement.shape = MaskShape::Star;
    replacement.inverted = true;
    s.apply(Command::UpdateMask {
        item: ItemId(2),
        mask: replacement,
    })
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
    s.apply(Command::RemoveMask {
        item: ItemId(2),
        mask: "a".into(),
    })
    .unwrap();
    assert!(
        s.project()
            .data()
            .scene()
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
    for command in [
        Command::AddMask {
            item: ItemId(2),
            mask: a,
        },
        Command::RemoveMask {
            item: ItemId(2),
            mask: "missing".into(),
        },
    ] {
        let before = s.project().clone();
        assert!(s.apply(command).is_err());
        assert_eq!(s.project(), &before);
    }
    for order in [vec!["a".to_owned()], vec!["a".to_owned(); 13]] {
        let before = s.project().clone();
        assert!(
            s.apply(Command::ReorderMasks {
                item: ItemId(2),
                order
            })
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
        s.apply(Command::RemoveMask {
            item: ItemId(2),
            mask: "a".into()
        })
        .is_err()
    );
    assert_eq!(s.project(), &before);
}

fn path_of(s: &Session, item: ItemId, id: &str) -> MaskPath {
    let item = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == item)
        .unwrap();
    let mask = item.masks.iter().find(|mask| mask.id == id).unwrap();
    match &mask.shape {
        MaskShape::Path { path } => path.clone(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn freeform_mask_path_commands_split_remove_and_restore_history_atomically() {
    let mut s = session();
    let mut seed = MaskPath::ellipse();
    seed.vertices[1].t_out = [0.0, 0.5];
    s.apply(Command::AddMask {
        item: ItemId(2),
        mask: Mask::new("p", MaskShape::Path { path: seed.clone() }),
    })
    .unwrap();
    assert_eq!(path_of(&s, ItemId(2), "p"), seed);

    // An equal path publishes no history step.
    let history = s.history_len();
    s.apply(Command::SetMaskPath {
        item: ItemId(2),
        mask: "p".into(),
        path: seed.clone(),
    })
    .unwrap();
    assert_eq!(s.history_len(), history);

    // Replacement preserves the other mask fields and other masks.
    let mut replacement = seed.clone();
    replacement.vertices[2].t_in = [0.25, 0.0];
    s.apply(Command::SetMaskPath {
        item: ItemId(2),
        mask: "p".into(),
        path: replacement.clone(),
    })
    .unwrap();
    let set = s.project().clone();
    assert_eq!(path_of(&s, ItemId(2), "p"), replacement);

    // A shape-preserving split inserts one anchor; undo/redo restores it.
    s.apply(Command::AddMaskPoint {
        item: ItemId(2),
        mask: "p".into(),
        segment: 1,
        t: 0.5,
    })
    .unwrap();
    let split = path_of(&s, ItemId(2), "p");
    assert_eq!(split.vertices.len(), 5);
    assert!(split.closed);
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &set);
    s.apply(Command::Redo {}).unwrap();
    assert_eq!(path_of(&s, ItemId(2), "p"), split);

    // Removing several anchors is one step and keeps survivor handles.
    let mut expected = split.clone();
    expected.vertices.remove(2);
    s.apply(Command::RemoveMaskPoint {
        item: ItemId(2),
        mask: "p".into(),
        points: vec![2],
    })
    .unwrap();
    assert_eq!(path_of(&s, ItemId(2), "p"), expected);
    let removed = s.project().clone();
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(path_of(&s, ItemId(2), "p"), split);
    s.apply(Command::Redo {}).unwrap();
    assert_eq!(s.project(), &removed);

    // Path edits round-trip through the saved project unchanged.
    let saved = project::encode(s.project(), false).unwrap();
    assert_eq!(
        &project::decode(&saved).unwrap().project,
        s.project().as_ref()
    );

    // Refusals leave the project and history untouched.
    for (command, label) in [
        (
            Command::AddMaskPoint {
                item: ItemId(2),
                mask: "p".into(),
                segment: 99,
                t: 0.5,
            },
            "segment",
        ),
        (
            Command::AddMaskPoint {
                item: ItemId(2),
                mask: "p".into(),
                segment: 0,
                t: 0.0,
            },
            "t low",
        ),
        (
            Command::AddMaskPoint {
                item: ItemId(2),
                mask: "missing".into(),
                segment: 0,
                t: 0.5,
            },
            "mask",
        ),
        (
            Command::RemoveMaskPoint {
                item: ItemId(2),
                mask: "p".into(),
                points: vec![0, 0],
            },
            "duplicate",
        ),
        (
            Command::RemoveMaskPoint {
                item: ItemId(2),
                mask: "p".into(),
                points: vec![0, 1, 2],
            },
            "minimum",
        ),
    ] {
        let before = s.project().clone();
        assert!(s.apply(command).is_err(), "{label}");
        assert_eq!(s.project(), &before, "{label}");
    }
    let mut open = seed.clone();
    open.closed = false;
    let mut open_mask = Mask::new("open", MaskShape::Path { path: open });
    open_mask.center_x = 0.1;
    s.apply(Command::AddMask {
        item: ItemId(2),
        mask: open_mask,
    })
    .unwrap();
    let before = s.project().clone();
    assert!(
        s.apply(Command::AddMaskPoint {
            item: ItemId(2),
            mask: "open".into(),
            segment: 0,
            t: 0.5,
        })
        .is_err()
    );
    assert!(
        s.apply(Command::RemoveMaskPoint {
            item: ItemId(2),
            mask: "open".into(),
            points: vec![0],
        })
        .is_err()
    );
    assert!(
        s.apply(Command::SetMaskPath {
            item: ItemId(2),
            mask: "open".into(),
            path: open_open_path(),
        })
        .is_err()
    );
    assert_eq!(s.project(), &before);
}

fn open_open_path() -> MaskPath {
    MaskPath {
        closed: false,
        vertices: MaskPath::ellipse().vertices,
    }
}
