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
    s.execute("keyframes.set_param",json!({"item":2,"path":"params.label","param":Param::new(ParamValue::Text("Một".into())).unwrap()})).unwrap();
    let mut label = Keyframe::new(Tick(0), ParamValue::Text("Hai".into()));
    label.segment = Segment::Step;
    s.execute(
        "keyframes.add",
        json!({"item":2,"path":"params.label","component":null,"key":label}),
    )
    .unwrap();
    assert_eq!(
        item(&s, ItemId(2))
            .animations
            .value_at(&AnimationPath::new("params.label").unwrap(), Tick(0))
            .unwrap(),
        Some(ParamValue::Text("Hai".into()))
    );
    s.execute(
        "keyframes.add",
        json!({"item":2,"path":"opacity","component":null,"key":key(60,0.4)}),
    )
    .unwrap();
    s.execute(
        "keyframes.retime",
        json!({"item":2,"path":"opacity","component":null,"from":frame(60),"to":frame(80)}),
    )
    .unwrap();
    let count = s.history_len().0;
    s.begin_gesture().unwrap();
    for dv in [0.1, 0.2, 0.3] {
        s.execute("keyframes.set_handle",json!({"item":2,"path":"opacity","component":null,"time":frame(80),"side":"out","handle":{"dt":frame(10),"dv":dv},"tangent":"aligned"})).unwrap();
    }
    s.commit_gesture().unwrap();
    assert_eq!(s.history_len().0, count + 1);
    let changed = s.project().clone();
    s.undo().unwrap();
    assert_ne!(s.project(), &changed);
    s.redo().unwrap();
    assert_eq!(s.project(), &changed);
    s.execute(
        "keyframes.remove",
        json!({"item":2,"path":"opacity","component":null,"time":frame(80)}),
    )
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
    s.execute(
        "keyframes.scale_time",
        json!({"item":2,"numerator":2,"denominator":1}),
    )
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
fn invalid_key_edits_and_locked_tracks_are_wholly_atomic() {
    let mut s = session();
    let original = s.project().clone();
    for command in [
        json!({"command":"keyframes.retime","params":{"item":2,"path":"opacity","component":null,"from":0,"to":frame(120)}}),
        json!({"command":"keyframes.set_handle","params":{"item":2,"path":"opacity","component":null,"time":0,"side":"in","handle":{"dt":1,"dv":0.0},"tangent":"broken"}}),
        json!({"command":"keyframes.scale_time","params":{"item":2,"numerator":1,"denominator":0}}),
        json!({"command":"keyframes.add","params":{"item":999,"path":"opacity","component":null,"key":key(30,0.3)}}),
        json!({"command":"keyframes.add","params":{"item":2,"path":"unknown","component":null,"key":key(30,0.3)}}),
    ] {
        let c: std::result::Result<Command, _> = serde_json::from_value(command);
        if let Ok(c) = c {
            assert!(s.apply(c).is_err());
        }
        assert_eq!(s.project(), &original);
        assert_eq!(s.history_len(), (0, 0));
    }
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
        s.execute(
            "keyframes.add",
            json!({"item":2,"path":"opacity","key":key(60,0.5)})
        )
        .is_err()
    );
    assert_eq!(s.project(), &before);
    assert!(s.execute("keyframes.set_handle",json!({"item":2,"path":"opacity","component":null,"time":0,"side":"out","tangent":"broken"})).is_err());
    assert_eq!(s.project(), &before);
}
