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
        items: vec![
            testkit::item::item(
                ItemId(2),
                ItemParams::Overlay(OverlayParams {
                    color: Color([255; 4]),
                }),
            )
            .source(SourceRange::new(Tick::ZERO, FrameRate::FPS_30.tick_of(30).unwrap()).unwrap())
            .build(),
        ],
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
        at: None,
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
                at: None,
            },
            "segment",
        ),
        (
            Command::AddMaskPoint {
                item: ItemId(2),
                mask: "p".into(),
                segment: 0,
                t: 0.0,
                at: None,
            },
            "t low",
        ),
        (
            Command::AddMaskPoint {
                item: ItemId(2),
                mask: "missing".into(),
                segment: 0,
                t: 0.5,
                at: None,
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
            at: None,
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

#[test]
fn mask_point_clipboard_rides_the_item_payload_and_pastes_in_one_undo_step() {
    use crate::support::session::Wire;
    use engine::{ItemClipboard, ItemPasteSource};
    use serde_json::json;
    let mut s = session();
    let seed = MaskPath::ellipse();
    for (id, path) in [("p", seed.clone()), ("q", MaskPath::rectangle())] {
        s.apply(Command::AddMask {
            item: ItemId(2),
            mask: Mask::new(id, MaskShape::Path { path }),
        })
        .unwrap();
    }
    s.apply(Command::AddMask {
        item: ItemId(2),
        mask: Mask::new("flat", MaskShape::Rectangle),
    })
    .unwrap();
    let copy = |mask: &str, points: Vec<usize>| Command::CopyMaskPoints {
        item: ItemId(2),
        mask: mask.into(),
        points,
    };

    // Nothing is stored for an empty selection; stale, duplicate or non-path targets refuse.
    assert!(!s.clipboard_has_entry());
    s.apply(copy("p", vec![])).unwrap();
    assert!(!s.clipboard_has_entry());
    assert!(!s.can_copy_mask_points(ItemId(2), "p", &[]));
    for (mask, points) in [
        ("p", vec![9]),
        ("p", vec![1, 1]),
        ("flat", vec![0]),
        ("none", vec![0]),
    ] {
        assert!(
            s.apply(copy(mask, points.clone())).is_err(),
            "{mask} {points:?}"
        );
        assert!(!s.can_copy_mask_points(ItemId(2), mask, &points));
        assert!(!s.clipboard_has_entry());
    }

    // Copy keeps path order whatever the selection order, and is not a history step.
    let history = s.history_len();
    assert!(s.can_copy_mask_points(ItemId(2), "p", &[3, 1]));
    let copied = s
        .wire(
            "clipboard.copy_mask_points",
            json!({"item":2,"mask":"p","points":[3,1]}),
        )
        .unwrap();
    assert!(!copied.changed);
    assert_eq!(s.history_len(), history);
    assert_eq!(s.mask_point_clipboard_len(), 2);
    assert!(
        !s.can_paste_items(Tick::ZERO),
        "an item Paste refuses mask points"
    );
    assert!(
        s.apply(Command::PasteItems {
            at: Tick::ZERO,
            source: ItemPasteSource::Stored,
        })
        .is_err()
    );

    // Paste lands the anchors after the given one with their tangents, in one undo step.
    assert!(s.can_paste_mask_points(ItemId(2), "q", 1));
    assert!(
        !s.can_paste_mask_points(ItemId(2), "q", 4),
        "past the last anchor"
    );
    assert!(!s.can_paste_mask_points(ItemId(2), "flat", 0));
    let before = s.project().clone();
    let paste = |after: usize| Command::PasteMaskPoints {
        item: ItemId(2),
        mask: "q".into(),
        after,
        source: ItemPasteSource::Stored,
    };
    s.apply(paste(1)).unwrap();
    let mut expected = MaskPath::rectangle();
    expected.vertices.insert(2, seed.vertices[3].clone());
    expected.vertices.insert(2, seed.vertices[1].clone());
    assert_eq!(path_of(&s, ItemId(2), "q"), expected);
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &before);
    s.apply(Command::Redo {}).unwrap();
    assert_eq!(path_of(&s, ItemId(2), "q"), expected);
    let pasted = s.project().clone();
    assert!(s.apply(paste(9)).is_err());
    assert_eq!(
        s.project(),
        &pasted,
        "a refusal leaves the project untouched"
    );

    // The explicit payload is the item payload with `maskPoints` and no items.
    let wire = json!({"version":1,"items":[],"span":0,"maskPoints":[
        {"p":[0.5,0.5],"t_in":[-0.1,0.0],"t_out":[0.1,0.0]}]});
    let payload: ItemClipboard = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(&payload).unwrap(), wire);
    assert_eq!(payload.mask_point_count(), 1);
    s.apply(Command::PasteMaskPoints {
        item: ItemId(2),
        mask: "p".into(),
        after: 3,
        source: ItemPasteSource::Payload { payload },
    })
    .unwrap();
    let grown = path_of(&s, ItemId(2), "p");
    assert_eq!(grown.vertices.len(), 5);
    assert_eq!(grown.vertices[4].t_in, [-0.1, 0.0]);
    let point = |p: f64| json!({"p":[p,0.0],"t_in":[0.0,0.0],"t_out":[0.0,0.0]});
    for bad in [
        json!({"version":1,"items":[],"span":0,"maskPoints":[]}),
        json!({"version":1,"items":[],"span":5,"maskPoints":[point(0.0)]}),
        json!({"version":1,"items":[],"span":0,"maskPoints":[point(101.0)]}),
        json!({"version":2,"items":[],"span":0,"maskPoints":[point(0.0)]}),
        json!({"version":1,"items":[],"span":0}),
    ] {
        assert!(
            serde_json::from_value::<ItemClipboard>(bad.clone()).is_err(),
            "{bad}"
        );
    }

    // The saved project never carries the clipboard.
    let saved = project::encode(s.project(), false).unwrap();
    assert_eq!(
        &project::decode(&saved).unwrap().project,
        s.project().as_ref()
    );
}

#[test]
fn mask_point_cut_copies_then_removes_the_anchors_in_one_undo_step() {
    use engine::{CutTarget, ItemPasteSource};
    let mut s = session();
    let seed = MaskPath::ellipse();
    s.apply(Command::AddMask {
        item: ItemId(2),
        mask: Mask::new("p", MaskShape::Path { path: seed.clone() }),
    })
    .unwrap();
    let target = |points: Vec<usize>| CutTarget::MaskPoints {
        item: ItemId(2),
        mask: "p".into(),
        points,
    };
    let cut = |points: Vec<usize>| Command::CutSelection {
        target: target(points),
    };

    // An empty selection is a no-op; a stale index refuses and stores nothing.
    let before = s.project().clone();
    let history = s.history_len();
    assert!(!s.can_cut(&target(vec![])));
    assert!(!s.apply(cut(vec![])).unwrap().changed);
    assert!(!s.can_cut(&target(vec![9])));
    assert!(s.apply(cut(vec![9])).is_err());
    assert_eq!(s.project(), &before);
    assert_eq!(s.history_len(), history);
    assert!(!s.clipboard_has_entry());

    // A cut that would leave fewer than three anchors refuses after the capture and
    // leaves the slot empty: the payload is stored only once the removal commits.
    assert!(s.apply(cut(vec![0, 1])).is_err());
    assert_eq!(s.project(), &before);
    assert!(!s.clipboard_has_entry());

    // Cut stores the anchor and removes it in one undo step.
    assert!(s.can_cut(&target(vec![1])));
    assert!(s.apply(cut(vec![1])).unwrap().changed);
    let mut expected = seed.clone();
    expected.vertices.remove(1);
    assert_eq!(path_of(&s, ItemId(2), "p"), expected);
    assert_eq!(s.history_len().0, history.0 + 1);
    assert_eq!(s.mask_point_clipboard_len(), 1);
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &before);
    assert_eq!(
        s.mask_point_clipboard_len(),
        1,
        "undo keeps the cut payload"
    );
    s.apply(Command::Redo {}).unwrap();

    // Paste after the cut puts the anchor back with its tangents.
    s.apply(Command::PasteMaskPoints {
        item: ItemId(2),
        mask: "p".into(),
        after: 0,
        source: ItemPasteSource::Stored,
    })
    .unwrap();
    assert_eq!(path_of(&s, ItemId(2), "p"), seed);

    // A locked track refuses the cut and leaves project, history and clipboard alone.
    s.apply(Command::SetTrackLocked {
        track: TrackId(1),
        locked: true,
    })
    .unwrap();
    let locked = s.project().clone();
    let history = s.history_len();
    assert!(!s.can_cut(&target(vec![2])));
    assert!(s.apply(cut(vec![2])).is_err());
    assert_eq!(s.project(), &locked);
    assert_eq!(s.history_len(), history);
    assert_eq!(s.mask_point_clipboard_len(), 1);
}

fn mask_keys(s: &Session, path: &AnimationPath) -> Vec<i64> {
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
        .get(path)
        .unwrap()
        .channel(None)
        .unwrap()
        .keys()
        .iter()
        .map(|key| key.time.0)
        .collect()
}

#[test]
fn mask_keys_toggle_undo_retime_and_survive_a_shape_change_and_refuse_ghosts() {
    let mut s = session();
    let a = Mask::new("a", MaskShape::Rectangle);
    s.apply(Command::AddMask {
        item: ItemId(2),
        mask: a.clone(),
    })
    .unwrap();
    let path = a.path("feather").unwrap();
    let before = s.project().clone();
    s.apply(Command::ToggleKeyframe {
        item: ItemId(2),
        path: path.clone(),
        component: None,
        time: Tick(12),
        value: ParamValue::Scalar(3.),
    })
    .unwrap();
    let keyed = s.project().clone();
    assert_eq!(mask_keys(&s, &path), vec![12]);
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &before);
    s.apply(Command::Redo {}).unwrap();
    assert_eq!(s.project(), &keyed);

    s.apply(Command::ScaleKeyframeTime {
        item: ItemId(2),
        numerator: 2,
        denominator: 1,
    })
    .unwrap();
    assert_eq!(mask_keys(&s, &path), vec![24]);

    // A shape change keeps the mask id, so its keys stay on the same path.
    let mut star = a.clone();
    star.shape = MaskShape::Star;
    s.apply(Command::UpdateMask {
        item: ItemId(2),
        mask: star,
    })
    .unwrap();
    assert_eq!(mask_keys(&s, &path), vec![24]);

    // A key on a mask the item does not carry refuses and leaves everything alone.
    let ghost = AnimationPath::new("masks.ghost.params.feather").unwrap();
    let before_ghost = s.project().clone();
    assert!(
        s.apply(Command::AddKeyframe {
            item: ItemId(2),
            path: ghost,
            component: None,
            key: Keyframe::new(Tick(1), ParamValue::Scalar(1.)),
        })
        .is_err()
    );
    assert_eq!(s.project(), &before_ghost);
}

fn has_channel(s: &Session, path: &AnimationPath) -> bool {
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
        .get(path)
        .is_some()
}

#[test]
fn freeform_anchor_keys_follow_point_edits_through_history_and_save() {
    use engine::{ItemClipboard, ItemPasteSource};
    let mut s = session();
    let mask = Mask::new(
        "p",
        MaskShape::Path {
            path: MaskPath::rectangle(),
        },
    );
    s.apply(Command::AddMask {
        item: ItemId(2),
        mask: mask.clone(),
    })
    .unwrap();
    let anchor = |index: usize, field: &str| mask.point_path(index, field).unwrap();
    let toggle = |path: AnimationPath, time: i64, value: f64| Command::ToggleKeyframe {
        item: ItemId(2),
        path,
        component: None,
        time: Tick(time),
        value: ParamValue::Scalar(value),
    };
    s.apply(toggle(anchor(1, "x"), 0, 1.0)).unwrap();
    s.apply(toggle(anchor(1, "x"), 30, 0.6)).unwrap();
    s.apply(toggle(anchor(3, "out_y"), 0, 0.2)).unwrap();
    s.apply(toggle(mask.path("feather").unwrap(), 0, 4.0))
        .unwrap();
    let keyed = s.project().clone();
    assert_eq!(mask_keys(&s, &anchor(1, "x")), [0, 30]);

    // Each anchor key is one undo step; redo restores them.
    s.apply(Command::Undo {}).unwrap();
    s.apply(Command::Undo {}).unwrap();
    assert!(!has_channel(&s, &anchor(3, "out_y")));
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(mask_keys(&s, &anchor(1, "x")), [0]);
    for _ in 0..3 {
        s.apply(Command::Redo {}).unwrap();
    }
    assert_eq!(s.project(), &keyed);
    // Removing an anchor key is one step too.
    s.apply(toggle(anchor(1, "x"), 30, 0.6)).unwrap();
    assert_eq!(mask_keys(&s, &anchor(1, "x")), [0]);
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &keyed);

    // Anchor keys survive save and reopen unchanged.
    let saved = project::encode(s.project(), false).unwrap();
    assert_eq!(
        &project::decode(&saved).unwrap().project,
        s.project().as_ref()
    );

    // A split of segment 0 inserts anchor 1: later channels move up with their keys.
    s.apply(Command::AddMaskPoint {
        item: ItemId(2),
        mask: "p".into(),
        segment: 0,
        t: 0.5,
        at: None,
    })
    .unwrap();
    // The new anchor is keyed at the split segment's key times; the keyed neighbour
    // moved up with its keys.
    assert_eq!(mask_keys(&s, &anchor(1, "x")), [0, 30]);
    assert_eq!(mask_keys(&s, &anchor(2, "x")), [0, 30]);
    assert_eq!(mask_keys(&s, &anchor(4, "out_y")), [0]);
    assert_eq!(mask_keys(&s, &mask.path("feather").unwrap()), [0]);
    let split = s.project().clone();
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &keyed);
    s.apply(Command::Redo {}).unwrap();
    assert_eq!(s.project(), &split);

    // Removing the keyed anchor drops its channel; the later one moves down.
    s.apply(Command::RemoveMaskPoint {
        item: ItemId(2),
        mask: "p".into(),
        points: vec![2],
    })
    .unwrap();
    assert!(!has_channel(&s, &anchor(2, "x")));
    assert!(!has_channel(&s, &anchor(4, "out_y")));
    assert_eq!(mask_keys(&s, &anchor(3, "out_y")), [0]);
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &split);

    // Two anchors pasted after anchor 0 move every later channel up by two.
    let wire = serde_json::json!({"version":1,"items":[],"span":0,"maskPoints":[
        {"p":[0.1,0.0],"t_in":[0.0,0.0],"t_out":[0.0,0.0]},
        {"p":[0.2,0.0],"t_in":[0.0,0.0],"t_out":[0.0,0.0]}]});
    let payload: ItemClipboard = serde_json::from_value(wire).unwrap();
    s.apply(Command::PasteMaskPoints {
        item: ItemId(2),
        mask: "p".into(),
        after: 0,
        source: ItemPasteSource::Payload { payload },
    })
    .unwrap();
    assert!(!has_channel(&s, &anchor(2, "x")));
    assert_eq!(mask_keys(&s, &anchor(4, "x")), [0, 30]);
    assert_eq!(mask_keys(&s, &anchor(6, "out_y")), [0]);
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &split);

    // A key on an anchor the path lacks refuses and changes nothing.
    let history = s.history_len();
    assert!(s.apply(toggle(anchor(5, "x"), 1, 1.0)).is_err());
    assert_eq!(s.project(), &split);
    assert_eq!(s.history_len(), history);

    // A shorter path keeps the channels it still carries; a non-path shape drops
    // them all while the scalar keys stay. Undo restores both.
    s.apply(Command::SetMaskPath {
        item: ItemId(2),
        mask: "p".into(),
        path: MaskPath::rectangle(),
    })
    .unwrap();
    assert_eq!(mask_keys(&s, &anchor(2, "x")), [0, 30]);
    assert!(!has_channel(&s, &anchor(4, "out_y")));
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &split);
    let mut rectangle = mask.clone();
    rectangle.shape = MaskShape::Rectangle;
    s.apply(Command::UpdateMask {
        item: ItemId(2),
        mask: rectangle,
    })
    .unwrap();
    assert!(!has_channel(&s, &anchor(2, "x")));
    assert!(!has_channel(&s, &anchor(4, "out_y")));
    assert_eq!(mask_keys(&s, &mask.path("feather").unwrap()), [0]);
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &split);

    // Removing the mask drops its anchor channels with its scalar ones.
    s.apply(Command::RemoveMask {
        item: ItemId(2),
        mask: "p".into(),
    })
    .unwrap();
    assert!(!has_channel(&s, &anchor(2, "x")));
    assert!(!has_channel(&s, &mask.path("feather").unwrap()));
}

fn evaluated(s: &Session, id: &str, time: i64) -> MaskPath {
    let item = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == ItemId(2))
        .unwrap();
    let mask = item.masks.iter().find(|mask| mask.id == id).unwrap();
    match mask.at(&item.animations, Tick(time)).unwrap().shape {
        MaskShape::Path { path } => path,
        other => panic!("{other:?}"),
    }
}

fn cubic(a: &MaskVertex, b: &MaskVertex, u: f64) -> [f64; 2] {
    let at = |axis: usize| {
        let p = [
            a.p[axis],
            a.p[axis] + a.t_out[axis],
            b.p[axis] + b.t_in[axis],
            b.p[axis],
        ];
        let v = 1.0 - u;
        v * v * v * p[0] + 3.0 * v * v * u * p[1] + 3.0 * v * u * u * p[2] + u * u * u * p[3]
    };
    [at(0), at(1)]
}

#[test]
fn splitting_an_animated_segment_keeps_its_curve_at_every_key_time() {
    let mut s = session();
    let mask = Mask::new(
        "p",
        MaskShape::Path {
            path: MaskPath::ellipse(),
        },
    );
    s.apply(Command::AddMask {
        item: ItemId(2),
        mask: mask.clone(),
    })
    .unwrap();
    let key = |index: usize, field: &str, time: i64, value: f64| Command::ToggleKeyframe {
        item: ItemId(2),
        path: mask.point_path(index, field).unwrap(),
        component: None,
        time: Tick(time),
        value: ParamValue::Scalar(value),
    };
    for command in [
        key(0, "out_x", 0, 0.1),
        key(0, "out_x", 30, 0.3),
        key(1, "in_y", 15, -0.25),
        key(1, "x", 30, 0.9),
        key(2, "out_x", 0, 0.5),
    ] {
        s.apply(command).unwrap();
    }
    let before: Vec<_> = [0, 15, 30]
        .into_iter()
        .map(|time| evaluated(&s, "p", time))
        .collect();
    let keyed = s.project().clone();
    let t = 0.25;
    s.apply(Command::AddMaskPoint {
        item: ItemId(2),
        mask: "p".into(),
        segment: 0,
        t,
        at: None,
    })
    .unwrap();

    // The first anchor's outgoing keys scale by t; the second anchor's incoming key moves
    // with its anchor and scales by 1 - t; the new anchor is keyed at every key time.
    let value = |index: usize, field: &str, time: i64| -> f64 {
        let item = s
            .project()
            .data()
            .scene()
            .timeline
            .overlays
            .first()
            .unwrap()
            .items
            .first()
            .unwrap();
        match item
            .animations
            .value_at(&mask.point_path(index, field).unwrap(), Tick(time))
            .unwrap()
        {
            Some(ParamValue::Scalar(v)) => v,
            other => panic!("{other:?}"),
        }
    };
    assert!((value(0, "out_x", 0) - 0.025).abs() < 1e-12);
    assert!((value(0, "out_x", 30) - 0.075).abs() < 1e-12);
    assert!((value(2, "in_y", 15) - 0.75 * -0.25).abs() < 1e-12);
    assert_eq!(
        mask_keys(&s, &mask.point_path(1, "x").unwrap()),
        [0, 15, 30]
    );
    assert_eq!(mask_keys(&s, &mask.point_path(3, "out_x").unwrap()), [0]);

    // At every key time the two new sub-curves trace the original segment.
    for (time, old) in [0, 15, 30].into_iter().zip(&before) {
        let new = evaluated(&s, "p", time);
        let (a, q, b) = (&old.vertices[0], &old.vertices[1], &new.vertices);
        for step in 0..=8 {
            let u = f64::from(step) / 8.0;
            let whole = cubic(a, q, u * t);
            let first = cubic(&b[0], &b[1], u);
            assert!((whole[0] - first[0]).abs() < 1e-9 && (whole[1] - first[1]).abs() < 1e-9);
            let whole = cubic(a, q, t + u * (1.0 - t));
            let second = cubic(&b[1], &b[2], u);
            assert!((whole[0] - second[0]).abs() < 1e-9 && (whole[1] - second[1]).abs() < 1e-9);
        }
    }

    // One undo step restores the keys and channels.
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &keyed);
}

#[test]
fn adding_a_point_at_a_mid_key_tick_lands_on_the_curve_evaluated_there() {
    let mut s = session();
    let mask = Mask::new(
        "p",
        MaskShape::Path {
            path: MaskPath::ellipse(),
        },
    );
    s.apply(Command::AddMask {
        item: ItemId(2),
        mask: mask.clone(),
    })
    .unwrap();
    for (index, field, time, value) in [
        (0, "out_x", 0, 0.1),
        (0, "out_x", 30, 0.4),
        (1, "x", 0, 1.0),
        (1, "x", 30, 0.8),
    ] {
        s.apply(Command::ToggleKeyframe {
            item: ItemId(2),
            path: mask.point_path(index, field).unwrap(),
            component: None,
            time: Tick(time),
            value: ParamValue::Scalar(value),
        })
        .unwrap();
    }
    let (middle, t) = (12, 0.4);
    let old = evaluated(&s, "p", middle);
    let expected = cubic(&old.vertices[0], &old.vertices[1], t);
    s.apply(Command::AddMaskPoint {
        item: ItemId(2),
        mask: "p".into(),
        segment: 0,
        t,
        at: Some(Tick(middle)),
    })
    .unwrap();
    let new = evaluated(&s, "p", middle);
    assert_eq!(new.vertices.len(), 5);
    assert!((new.vertices[1].p[0] - expected[0]).abs() < 1e-9);
    assert!((new.vertices[1].p[1] - expected[1]).abs() < 1e-9);
    assert_eq!(
        mask_keys(&s, &mask.point_path(1, "x").unwrap()),
        [0, 12, 30]
    );
}
