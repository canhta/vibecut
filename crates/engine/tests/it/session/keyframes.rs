#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "validated keyframe fixtures"
)]
use engine::{Command, Session, registry};
use project::*;
use serde_json::json;
use time::{FrameRate, SourceRange, Tick};
fn frame(n: i64) -> Tick {
    FrameRate::FPS_29_97.tick_of(n).unwrap()
}
fn path() -> AnimationPath {
    AnimationPath::new("opacity").unwrap()
}
fn key(n: i64, v: f64) -> Keyframe {
    Keyframe::new(frame(n), ParamValue::Scalar(v))
}
fn animated() -> Animations {
    let mut a = key(0, 0.0);
    a.segment = Segment::Bezier;
    a.outgoing = Some(CurveHandle {
        dt: frame(40),
        dv: 0.1,
    });
    let mut b = key(120, 1.0);
    b.incoming = Some(CurveHandle {
        dt: frame(-30),
        dv: -0.2,
    });
    let c = Channel::new(
        ParamValue::Scalar(0.0),
        vec![a, b],
        Extrapolation::Linear,
        Extrapolation::Hold,
    )
    .unwrap();
    let mut animations = Animations::default();
    animations.set(path(), Param::Value(c)).unwrap();
    animations
}
fn session() -> Session {
    let mut data = Project::empty(ProjectId(1), "Đường cong".into())
        .unwrap()
        .into_data();
    data.canvas.fps = FrameRate::FPS_29_97;
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(2),
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
            effects: Vec::new(),
            id: ItemId(2),
            source_asset: None,
            source_range: SourceRange::new(frame(50), frame(650)).unwrap(),
            start: frame(100),
            content: ItemParams::Overlay(OverlayParams {
                color: Color([255; 4]),
            }),
            animations: animated(),
        }],
    });
    Session::new(Project::new(data).unwrap(), 100).unwrap()
}
fn item(s: &Session, id: ItemId) -> &Item {
    s.project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == id)
        .unwrap()
}
fn value(a: &Animations, t: Tick) -> f64 {
    if let Some(ParamValue::Scalar(v)) = a.value_at(&path(), t).unwrap() {
        v
    } else {
        panic!("scalar")
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-8, "{a} != {b}");
}

#[test]
fn transform_and_graph_edits_share_the_timeline_preview_transaction() {
    let mut s = session();
    let before = s.project().clone();
    s.apply(Command::TimelinePreview {
        commands: vec![
            Command::SetItemField {
                item: ItemId(2),
                patch: engine::ItemParamPatch::PositionX(120.0),
            },
            Command::RetimeKeyframe {
                item: ItemId(2),
                path: path(),
                component: None,
                from: frame(120),
                to: frame(100),
            },
        ],
    })
    .unwrap();
    assert_eq!(s.project(), &before);
    let candidate = s.preview_project().unwrap();
    let changed = candidate
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == ItemId(2))
        .unwrap();
    assert_eq!(
        TransformField::PositionX
            .value_at(&changed.animations, Tick::ZERO)
            .unwrap(),
        120.0
    );
    assert_eq!(
        changed
            .animations
            .get(&path())
            .unwrap()
            .channel(None)
            .unwrap()
            .keys()[1]
            .time,
        frame(100)
    );
    s.apply(Command::TimelineDiscardPreview {}).unwrap();
    assert_eq!(s.project(), &before);
    assert_eq!(s.history_len(), (0, 0));
}
#[test]
fn registered_headless_edits_undo_redo_and_one_step_per_gesture() {
    let mut s = session();
    let before = s.project().clone();
    for id in [
        "keyframes.set_param",
        "keyframes.add",
        "keyframes.remove",
        "keyframes.retime",
        "keyframes.set_handle",
        "keyframes.scale_time",
    ] {
        assert!(registry().iter().find(|r| r.id == id).unwrap().enabled(&s));
    }
    s.apply(Command::SetAnimatedParam {
        item: ItemId(2),
        path: AnimationPath::new("params.label").unwrap(),
        param: Param::new(ParamValue::Text("Một".into())).unwrap(),
    })
    .unwrap();
    let mut label = Keyframe::new(Tick(0), ParamValue::Text("Hai".into()));
    label.segment = Segment::Step;
    s.apply(Command::AddKeyframe {
        item: ItemId(2),
        path: AnimationPath::new("params.label").unwrap(),
        component: None,
        key: label,
    })
    .unwrap();
    assert_eq!(
        item(&s, ItemId(2))
            .animations
            .value_at(&AnimationPath::new("params.label").unwrap(), Tick(0))
            .unwrap(),
        Some(ParamValue::Text("Hai".into()))
    );
    s.apply(Command::AddKeyframe {
        item: ItemId(2),
        path: path(),
        component: None,
        key: key(60, 0.4),
    })
    .unwrap();
    s.apply(Command::RetimeKeyframe {
        item: ItemId(2),
        path: path(),
        component: None,
        from: frame(60),
        to: frame(80),
    })
    .unwrap();
    let count = s.history_len().0;
    s.begin_gesture().unwrap();
    for dv in [0.1, 0.2, 0.3] {
        s.apply(Command::SetKeyframeHandle {
            item: ItemId(2),
            path: path(),
            component: None,
            time: frame(80),
            side: HandleSide::Out,
            handle: Some(CurveHandle { dt: frame(10), dv }),
            tangent: TangentMode::Aligned,
        })
        .unwrap();
    }
    s.commit_gesture().unwrap();
    assert_eq!(s.history_len().0, count + 1);
    let changed = s.project().clone();
    s.undo().unwrap();
    assert_ne!(s.project(), &changed);
    s.redo().unwrap();
    assert_eq!(s.project(), &changed);
    s.apply(Command::RemoveKeyframe {
        item: ItemId(2),
        path: path(),
        component: None,
        time: frame(80),
    })
    .unwrap();
    while s.can_undo() {
        s.undo().unwrap();
    }
    assert_eq!(s.project(), &before);
    while s.can_redo() {
        s.redo().unwrap();
    }
    assert_ne!(s.project(), &before);
}
#[test]
fn toggle_at_adds_then_removes_one_key_and_keeps_the_base() {
    let mut s = session();
    let before = item(&s, ItemId(2)).animations.get(&path()).cloned();
    let time = frame(40);
    s.apply(Command::ToggleKeyframe {
        item: ItemId(2),
        path: path(),
        component: None,
        time,
        value: ParamValue::Scalar(0.25),
    })
    .unwrap();
    let channel = item(&s, ItemId(2))
        .animations
        .get(&path())
        .unwrap()
        .channel(None)
        .unwrap();
    assert_eq!(channel.keys().len(), 3);
    assert_eq!(
        channel
            .keys()
            .iter()
            .find(|key| key.time == time)
            .map(|key| &key.value),
        Some(&ParamValue::Scalar(0.25))
    );
    s.apply(Command::ToggleKeyframe {
        item: ItemId(2),
        path: path(),
        component: None,
        time,
        value: ParamValue::Scalar(0.9),
    })
    .unwrap();
    let channel = item(&s, ItemId(2))
        .animations
        .get(&path())
        .unwrap()
        .channel(None)
        .unwrap();
    assert_eq!(channel.keys().len(), 2);
    assert_eq!(item(&s, ItemId(2)).animations.get(&path()), before.as_ref());
}

#[test]
fn delete_selection_removes_every_listed_key_atomically() {
    let mut s = session();
    let history = s.history_len().0;
    let keys = vec![
        engine::KeyframeRef {
            item: ItemId(2),
            path: path(),
            component: None,
            time: frame(0),
        },
        engine::KeyframeRef {
            item: ItemId(2),
            path: path(),
            component: None,
            time: frame(120),
        },
    ];
    s.apply(Command::DeleteKeyframes { keys }).unwrap();
    assert!(
        item(&s, ItemId(2))
            .animations
            .get(&path())
            .unwrap()
            .channel(None)
            .unwrap()
            .keys()
            .is_empty()
    );
    assert_eq!(s.history_len().0, history + 1);
    let stale = Command::DeleteKeyframes {
        keys: vec![engine::KeyframeRef {
            item: ItemId(2),
            path: path(),
            component: None,
            time: frame(0),
        }],
    };
    assert!(s.apply(stale).is_err());
    assert_eq!(s.history_len().0, history + 1, "a stale list adds no step");
    s.undo().unwrap();
    let key_ref = |item, time| engine::KeyframeRef {
        item: ItemId(item),
        path: path(),
        component: None,
        time,
    };
    let hostile = [
        ("empty selection", Command::DeleteKeyframes { keys: vec![] }),
        (
            "duplicate entry refuses the whole list",
            Command::DeleteKeyframes {
                keys: vec![key_ref(2, frame(0)), key_ref(2, frame(0))],
            },
        ),
        (
            "one stale entry spares the valid one",
            Command::DeleteKeyframes {
                keys: vec![key_ref(2, frame(0)), key_ref(2, frame(7))],
            },
        ),
        (
            "unknown item",
            Command::DeleteKeyframes {
                keys: vec![key_ref(99, frame(0))],
            },
        ),
    ];
    let history = s.history_len().0;
    let project = s.project().clone();
    for (case, command) in hostile {
        assert!(s.apply(command).is_err(), "{case}");
        assert_eq!(s.history_len().0, history, "{case}");
        assert_eq!(s.project(), &project, "{case}");
    }
    s.apply(Command::SetTrackLocked {
        track: TrackId(2),
        locked: true,
    })
    .unwrap();
    let locked = s.history_len().0;
    let project = s.project().clone();
    assert!(
        s.apply(Command::DeleteKeyframes {
            keys: vec![key_ref(2, frame(0))]
        })
        .is_err(),
        "locked track"
    );
    assert!(
        s.apply(Command::ToggleKeyframe {
            item: ItemId(2),
            path: path(),
            component: None,
            time: frame(40),
            value: ParamValue::Scalar(0.5),
        })
        .is_err(),
        "locked track"
    );
    assert!(
        s.apply(Command::SetKeyframeExtrapolation {
            item: ItemId(2),
            path: path(),
            component: None,
            before: Extrapolation::Hold,
            after: Extrapolation::Hold,
        })
        .is_err(),
        "locked track"
    );
    assert_eq!((s.history_len().0, s.project()), (locked, &project));
    s.apply(Command::SetTrackLocked {
        track: TrackId(2),
        locked: false,
    })
    .unwrap();
    assert!(
        s.apply(Command::ToggleKeyframe {
            item: ItemId(2),
            path: path(),
            component: None,
            time: frame(40),
            value: ParamValue::Bool(true),
        })
        .is_err(),
        "value kind mismatch"
    );
    assert_eq!(
        item(&s, ItemId(2))
            .animations
            .get(&path())
            .unwrap()
            .channel(None)
            .unwrap()
            .keys()
            .len(),
        2
    );
}

#[test]
fn extrapolation_choice_persists_with_the_item_and_refuses_discrete_values() {
    let mut s = session();
    s.apply(Command::SetKeyframeExtrapolation {
        item: ItemId(2),
        path: path(),
        component: None,
        before: Extrapolation::Hold,
        after: Extrapolation::Linear,
    })
    .unwrap();
    let channel = item(&s, ItemId(2))
        .animations
        .get(&path())
        .unwrap()
        .channel(None)
        .unwrap();
    assert_eq!(
        channel.extrapolation(),
        (Extrapolation::Hold, Extrapolation::Linear)
    );
    let encoded = project::encode(s.project(), false).unwrap();
    let decoded = project::decode(&encoded).unwrap();
    assert_eq!(decoded.project, **s.project());
    let bool_path = AnimationPath::new("muted").unwrap();
    s.apply(Command::SetAnimatedParam {
        item: ItemId(2),
        path: bool_path.clone(),
        param: Param::new(ParamValue::Bool(true)).unwrap(),
    })
    .unwrap();
    assert!(
        s.apply(Command::SetKeyframeExtrapolation {
            item: ItemId(2),
            path: bool_path.clone(),
            component: None,
            before: Extrapolation::Linear,
            after: Extrapolation::Hold,
        })
        .is_err(),
        "a discrete channel holds outside its keys"
    );
    assert!(item(&s, ItemId(2)).animations.get(&bool_path).is_some());
}

#[test]
fn trim_split_and_move_preserve_curves_at_2997_without_tick_drift() {
    let mut s = session();
    let original = item(&s, ItemId(2)).clone();
    s.apply(Command::TrimStart {
        item: ItemId(2),
        at: frame(110),
    })
    .unwrap();
    let trimmed = item(&s, ItemId(2));
    assert_eq!(
        trimmed
            .animations
            .get(&path())
            .unwrap()
            .channel(None)
            .unwrap()
            .keys()
            .first()
            .unwrap()
            .time,
        frame(-10)
    );
    for n in 10..600 {
        near(
            value(&trimmed.animations, frame(n - 10)),
            value(&original.animations, frame(n)),
        );
    }
    s.apply(Command::TrimEnd {
        item: ItemId(2),
        at: frame(690),
    })
    .unwrap();
    s.apply(Command::SplitAt {
        item: ItemId(2),
        at: frame(200),
    })
    .unwrap();
    let items = &s
        .project()
        .data()
        .scene()
        .timeline
        .overlays
        .first()
        .unwrap()
        .items;
    let right = items.get(1).unwrap();
    let right_id = right.id;
    assert_eq!(right.source_range.start(), frame(150));
    for n in 10..590 {
        let clip = if n < 100 {
            item(&s, ItemId(2))
        } else {
            item(&s, right_id)
        };
        near(
            value(
                &clip.animations,
                frame(n + 100).checked_sub(clip.start).unwrap(),
            ),
            value(&original.animations, frame(n)),
        );
    }
    let right_before = item(&s, right_id).animations.clone();
    s.apply(Command::MoveItem {
        item: right_id,
        track: TrackId(2),
        start: frame(800),
    })
    .unwrap();
    assert_eq!(item(&s, right_id).animations, right_before);
    s.undo().unwrap();
    s.undo().unwrap();
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(item(&s, ItemId(2)), &original);
}
#[test]
fn trim_gesture_reversal_and_cancel_keep_original_curves_and_history() {
    let mut s = session();
    let original = s.project().clone();
    s.begin_gesture().unwrap();
    for n in [120, 105, 150, 100] {
        s.apply(Command::TrimStart {
            item: ItemId(2),
            at: frame(n),
        })
        .unwrap();
    }
    s.commit_gesture().unwrap();
    assert_eq!(s.project(), &original);
    assert_eq!(s.history_len(), (0, 0));
    s.begin_gesture().unwrap();
    s.apply(Command::TrimStart {
        item: ItemId(2),
        at: frame(175),
    })
    .unwrap();
    s.cancel_gesture().unwrap();
    assert_eq!(s.project(), &original);
    s.begin_gesture().unwrap();
    s.apply(Command::TrimStart {
        item: ItemId(2),
        at: frame(120),
    })
    .unwrap();
    s.apply(Command::TrimStart {
        item: ItemId(2),
        at: frame(140),
    })
    .unwrap();
    s.commit_gesture().unwrap();
    assert_eq!(s.history_len(), (1, 0));
    for n in 40..600 {
        near(
            value(&item(&s, ItemId(2)).animations, frame(n - 40)),
            value(&item_from_project(&original).animations, frame(n)),
        );
    }
    s.undo().unwrap();
    assert_eq!(s.project(), &original);
    s.redo().unwrap();
}
fn item_from_project(p: &Project) -> &Item {
    p.data()
        .scene()
        .timeline
        .overlays
        .first()
        .unwrap()
        .items
        .first()
        .unwrap()
}
#[test]
fn scale_time_preserves_handles_values_serde_and_clean_undo() {
    let mut s = session();
    let original = s.project().clone();
    s.apply(Command::ScaleKeyframeTime {
        item: ItemId(2),
        numerator: 2,
        denominator: 1,
    })
    .unwrap();
    let scaled = item(&s, ItemId(2));
    for n in 0..200 {
        near(
            value(&scaled.animations, frame(n * 2)),
            value(&item_from_project(&original).animations, frame(n)),
        );
    }
    assert_eq!(
        scaled
            .animations
            .get(&path())
            .unwrap()
            .channel(None)
            .unwrap()
            .keys()
            .first()
            .unwrap()
            .outgoing
            .unwrap()
            .dt,
        frame(80)
    );
    let bytes = encode(s.project(), true).unwrap();
    let loaded = decode(&bytes).unwrap();
    assert_eq!(&loaded.project, s.project().as_ref());
    assert_eq!(loaded.schema_version, project::SCHEMA_VERSION);
    let mut old: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    old["schema_version"] = json!(1);
    assert!(matches!(
        decode(&serde_json::to_vec(&old).unwrap()),
        Err(Error::TooOld {
            found: 1,
            supported: project::SCHEMA_VERSION
        })
    ));
    s.undo().unwrap();
    assert_eq!(s.project(), &original);
    s.redo().unwrap();
}

#[test]
fn visual_parameters_validate_atomically_and_roundtrip_history_and_animation() {
    let mut s = session();
    let original = s.project().clone();
    for mode in BlendMode::ALL {
        let before = s.project().clone();
        s.apply(Command::SetItemField {
            item: ItemId(2),
            patch: engine::ItemParamPatch::BlendMode(mode),
        })
        .unwrap();
        assert_eq!(item(&s, ItemId(2)).blend_mode, mode);
        if s.project() != &before {
            let after = s.project().clone();
            s.undo().unwrap();
            assert_eq!(s.project(), &before);
            s.redo().unwrap();
            assert_eq!(s.project(), &after);
        }
        let bytes = encode(s.project(), false).unwrap();
        assert_eq!(&decode(&bytes).unwrap().project, s.project().as_ref());
    }
    s.apply(Command::SetItemField {
        item: ItemId(2),
        patch: engine::ItemParamPatch::BlendMode(BlendMode::Screen),
    })
    .unwrap();
    let curved = item(&s, ItemId(2)).clone();
    s.apply(Command::TrimStart {
        item: ItemId(2),
        at: frame(120),
    })
    .unwrap();
    assert_eq!(item(&s, ItemId(2)).blend_mode, BlendMode::Screen);
    for n in [0, 10, 50, 120] {
        near(
            value(&item(&s, ItemId(2)).animations, frame(n)),
            value(&curved.animations, frame(n + 20)),
        );
    }
    let trimmed = item(&s, ItemId(2)).clone();
    s.apply(Command::ScaleKeyframeTime {
        item: ItemId(2),
        numerator: 2,
        denominator: 1,
    })
    .unwrap();
    assert_eq!(item(&s, ItemId(2)).blend_mode, BlendMode::Screen);
    for n in [0, 10, 50, 120] {
        near(
            value(&item(&s, ItemId(2)).animations, frame(n * 2)),
            value(&trimmed.animations, frame(n)),
        );
    }
    let before = s.project().clone();
    let history = s.history_len();
    for patch in [
        engine::ItemParamPatch::Opacity(-0.01),
        engine::ItemParamPatch::Opacity(1.01),
    ] {
        assert!(
            s.apply(Command::SetItemField {
                item: ItemId(2),
                patch
            })
            .is_err()
        );
        assert_eq!(s.project(), &before);
        assert_eq!(s.history_len(), history);
    }
    assert!(
        crate::wire::run(
            &mut s,
            "timeline.set_item_param",
            json!({"item":2,"patch":{"field":"blendMode","value":"unknown"}})
        )
        .is_err()
    );
    assert_eq!(s.project(), &before);
    assert_eq!(s.history_len(), history);
    for opacity in [0., 0.5, 1.] {
        let before = s.project().clone();
        s.apply(Command::SetItemField {
            item: ItemId(2),
            patch: engine::ItemParamPatch::Opacity(opacity),
        })
        .unwrap();
        near(value(&item(&s, ItemId(2)).animations, Tick::ZERO), opacity);
        assert_eq!(
            &decode(&encode(s.project(), false).unwrap())
                .unwrap()
                .project,
            s.project().as_ref()
        );
        let after = s.project().clone();
        s.undo().unwrap();
        assert_eq!(s.project(), &before);
        s.redo().unwrap();
        assert_eq!(s.project(), &after);
    }
    assert_ne!(s.project(), &original);
    let before = s.project().clone();
    s.apply(Command::SetTrackFlags {
        track: TrackId(2),
        muted: false,
        hidden: false,
        locked: true,
    })
    .unwrap();
    assert!(
        s.apply(Command::SetItemField {
            item: ItemId(2),
            patch: engine::ItemParamPatch::Opacity(0.5)
        })
        .is_err()
    );
    s.undo().unwrap();
    assert_eq!(s.project(), &before);
}
#[test]
fn invalid_key_edits_and_locked_tracks_are_wholly_atomic() {
    let mut s = session();
    let original = s.project().clone();
    for command in [
        Command::RetimeKeyframe {
            item: ItemId(2),
            path: path(),
            component: None,
            from: Tick(0),
            to: frame(120),
        },
        Command::SetKeyframeHandle {
            item: ItemId(2),
            path: path(),
            component: None,
            time: Tick(0),
            side: HandleSide::In,
            handle: Some(CurveHandle {
                dt: Tick(1),
                dv: 0.0,
            }),
            tangent: TangentMode::Broken,
        },
        Command::ScaleKeyframeTime {
            item: ItemId(2),
            numerator: 1,
            denominator: 0,
        },
        Command::AddKeyframe {
            item: ItemId(999),
            path: path(),
            component: None,
            key: key(30, 0.3),
        },
    ] {
        assert!(s.apply(command).is_err());
        assert_eq!(s.project(), &original);
        assert_eq!(s.history_len(), (0, 0));
    }
    assert!(
        crate::wire::run(
            &mut s,
            "keyframes.add",
            json!({"item":2,"path":"unknown","component":null,"key":key(30,0.3)})
        )
        .is_err()
    );
    assert_eq!(s.project(), &original);
    assert_eq!(s.history_len(), (0, 0));
    s.apply(Command::SetTrackFlags {
        track: TrackId(2),
        muted: false,
        hidden: false,
        locked: true,
    })
    .unwrap();
    let locked = s.project().clone();
    assert!(
        !registry()
            .iter()
            .find(|r| r.id == "keyframes.add")
            .unwrap()
            .enabled(&s)
    );
    assert!(
        s.apply(Command::AddKeyframe {
            item: ItemId(2),
            path: path(),
            component: None,
            key: key(30, 0.3)
        })
        .is_err()
    );
    assert_eq!(s.project(), &locked);
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config {
        rng_seed: proptest::test_runner::RngSeed::Fixed(0x38),
        failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::Direct("tests/keyframes.proptest-regressions"))),
        ..proptest::test_runner::Config::default()
    })]

    #[test]
    fn random_ntsc_trim_split_retime_keep_exact_local_times(trim in 0i64..100,cut in 101i64..500,sample in 0i64..600,scale in 1i64..8) {
        let mut s=session();let original=item(&s,ItemId(2)).animations.clone();
        s.apply(Command::TrimStart{item:ItemId(2),at:frame(100+trim)}).unwrap();
        s.apply(Command::SplitAt{item:ItemId(2),at:frame(100+cut)}).unwrap();
        let time=trim+sample%(600-trim);
        let selected=s.project().data().scene().timeline.overlays.first().unwrap().items.iter().find(|i|frame(100+time)>=i.start && frame(100+time)<i.end().unwrap()).unwrap();
        let id=selected.id;let local=frame(100+time).checked_sub(selected.start).unwrap();
        let expected=value(&original,frame(time));
        proptest::prop_assert!((value(&selected.animations,local)-expected).abs()<1e-8);
        s.apply(Command::ScaleKeyframeTime{item:id,numerator:scale,denominator:1}).unwrap();
        let scaled=local.checked_scale(scale,1,time::Rounding::Nearest).unwrap();
        proptest::prop_assert!((value(&item(&s,id).animations,scaled)-expected).abs()<1e-8);
    }
}

#[test]
fn nullable_command_fields_are_still_required_in_json() {
    let mut s = session();
    let before = s.project().clone();
    assert!(
        crate::wire::run(
            &mut s,
            "keyframes.add",
            json!({"item":2,"path":"opacity","key":key(60,0.5)})
        )
        .is_err()
    );
    assert_eq!(s.project(), &before);
    assert!(
        crate::wire::run(
            &mut s,
            "keyframes.set_handle",
            json!({"item":2,"path":"opacity","component":null,"time":0,"side":"out","tangent":"broken"})
        )
        .is_err()
    );
    assert_eq!(s.project(), &before);
}
#[test]
fn text_background_channels_survive_trim_retime_and_the_file() {
    let scalar = |from: f64, to: f64| {
        Channel::new(
            ParamValue::Scalar(from),
            vec![
                Keyframe::new(frame(100), ParamValue::Scalar(from)),
                Keyframe::new(frame(400), ParamValue::Scalar(to)),
            ],
            Extrapolation::Hold,
            Extrapolation::Hold,
        )
        .unwrap()
    };
    let mut animations = Animations::default();
    animations
        .set(
            AnimationPath::new("background.offsetX").unwrap(),
            Param::Value(scalar(0.0, 40.0)),
        )
        .unwrap();
    let rgba = [(1.0, 0.0), (0.0, 0.0), (0.0, 1.0), (1.0, 1.0)];
    animations
        .set(
            AnimationPath::new("background.color").unwrap(),
            Param::Color(Box::new(rgba.map(|(from, to)| scalar(from, to)))),
        )
        .unwrap();
    let mut s = session();
    {
        // Replace the fixture's overlay with a text item carrying the background channels.
        let mut data = s.project().as_ref().clone().into_data();
        data.scene_mut().timeline.overlays.first_mut().unwrap().kind = TrackKind::Text;
        let item = data
            .scene_mut()
            .timeline
            .iter_mut()
            .flat_map(|t| &mut t.items)
            .find(|i| i.id == ItemId(2))
            .unwrap();
        item.content = ItemParams::Text(TextParams {
            background: Some(TextBackground::default()),
            ..TextParams::new("Xin chào")
        });
        item.animations = animations;
        s = Session::new(Project::new(data).unwrap(), 100).unwrap();
    }
    let offset = AnimationPath::new("background.offsetX").unwrap();
    let colour = AnimationPath::new("background.color").unwrap();
    let times = |s: &Session, path: &AnimationPath, component| {
        item(s, ItemId(2))
            .animations
            .get(path)
            .unwrap()
            .channel(component)
            .unwrap()
            .keys()
            .iter()
            .map(|k| k.time)
            .collect::<Vec<_>>()
    };
    let original = s.project().clone();
    s.apply(Command::TrimStart {
        item: ItemId(2),
        at: frame(110),
    })
    .unwrap();
    // The item start moved 10 frames; keys keep their exact source-time curve.
    assert_eq!(times(&s, &offset, None), [frame(90), frame(390)]);
    assert_eq!(
        times(&s, &colour, Some(ColorComponent::B)),
        [frame(90), frame(390)]
    );
    s.apply(Command::ScaleKeyframeTime {
        item: ItemId(2),
        numerator: 2,
        denominator: 1,
    })
    .unwrap();
    assert_eq!(times(&s, &offset, None), [frame(180), frame(780)]);
    let bytes = encode(s.project(), true).unwrap();
    assert_eq!(&decode(&bytes).unwrap().project, s.project().as_ref());
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(s.project(), &original);
}
