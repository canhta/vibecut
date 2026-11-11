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

fn title(base: Option<ParamValue>, length: i64) -> Session {
    let seed = crate::support::control::with_text(&crate::support::control::seed());
    let mut data = seed.project().data().clone();
    data.canvas.fps = FrameRate::FPS_29_97;
    let target = data
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|t| &mut t.items)
        .find(|i| i.id == ItemId(3))
        .unwrap();
    target.start = frame(100);
    target.source_range =
        SourceRange::new(frame(50), frame(50).checked_add(Tick(length)).unwrap()).unwrap();
    if let Some(base) = base {
        target
            .animations
            .set(path(), Param::new(base).unwrap())
            .unwrap();
    }
    Session::new(Project::new(data).unwrap(), 100).unwrap()
}

#[test]
fn text_animation_expands_literal_keys_preserves_base_and_exact_noop_redo() {
    for (length, d, exit) in [
        (254_016_000_000, 76_204_800_000, 177_811_200_000),
        (16_951_334_400, 8_475_667_200, 8_475_667_200),
        (16_951_334_401, 8_475_667_200, 8_475_667_201),
    ] {
        for base in [None, Some(0.0), Some(0.6)] {
            let b = base.unwrap_or(1.0);
            for (mode, expected) in [
                (engine::TextAnimationPreset::FadeIn, vec![(0, 0.0), (d, b)]),
                (
                    engine::TextAnimationPreset::FadeOut,
                    vec![(exit, b), (length, 0.0)],
                ),
                (
                    engine::TextAnimationPreset::FadeInOut,
                    if d == exit {
                        vec![(0, 0.0), (d, b), (length, 0.0)]
                    } else {
                        vec![(0, 0.0), (d, b), (exit, b), (length, 0.0)]
                    },
                ),
            ] {
                let mut s = title(base.map(ParamValue::Scalar), length);
                let before = s.project().clone();
                let content = item(&s, ItemId(3)).content.clone();
                let command = || Command::ApplyTextAnimation {
                    item: ItemId(3),
                    preset: mode,
                };
                assert!(s.apply(command()).unwrap().changed);
                let channel = item(&s, ItemId(3))
                    .animations
                    .get(&path())
                    .unwrap()
                    .channel(None)
                    .unwrap();
                assert_eq!(channel.base(), &ParamValue::Scalar(b));
                assert_eq!(
                    channel.extrapolation(),
                    (Extrapolation::Hold, Extrapolation::Hold)
                );
                assert_eq!(
                    channel.keys(),
                    expected
                        .iter()
                        .map(|(t, v)| Keyframe::new(Tick(*t), ParamValue::Scalar(*v)))
                        .collect::<Vec<_>>()
                );
                assert_eq!(item(&s, ItemId(3)).content, content);
                assert_eq!(item(&s, ItemId(3)).start, frame(100));
                assert_eq!(item(&s, ItemId(3)).source_range.start(), frame(50));
                near(
                    value(&item(&s, ItemId(3)).animations, Tick(d / 2)),
                    if mode == engine::TextAnimationPreset::FadeOut {
                        b
                    } else {
                        b / 2.0
                    },
                );
                let saved = encode(s.project(), true).unwrap();
                assert_eq!(decode(&saved).unwrap().project, **s.project());
                s.undo().unwrap();
                assert_eq!(s.project(), &before);
                s.redo().unwrap();
                s.apply(Command::RenameProject {
                    name: "Later".into(),
                })
                .unwrap();
                s.undo().unwrap();
                s.take_change();
                let snapshot = s.project().clone();
                let rev = s.revision();
                let history = s.history_len();
                assert!(!s.apply(command()).unwrap().changed);
                assert!(std::sync::Arc::ptr_eq(&snapshot, s.project()));
                assert_eq!((s.revision(), s.history_len()), (rev, history));
                assert!(s.take_change().is_none());
                let keys: Vec<_> = expected
                    .iter()
                    .map(|(t, _)| engine::KeyframeRef {
                        item: ItemId(3),
                        path: path(),
                        component: None,
                        time: Tick(*t),
                    })
                    .collect();
                s.apply(Command::DeleteKeyframes { keys }).unwrap();
                near(value(&item(&s, ItemId(3)).animations, Tick(d)), b);
            }
        }
    }
}
#[test]
fn text_animation_refusals_and_transactions_preserve_all_state() {
    use engine::TextAnimationPreset::FadeInOut;
    let apply = || Command::ApplyTextAnimation {
        item: ItemId(3),
        preset: FadeInOut,
    };
    for (base, length) in [
        (Some(ParamValue::Scalar(-0.1)), 254_016_000_000),
        (Some(ParamValue::Scalar(1.1)), 254_016_000_000),
        (None, 16_951_334_399),
    ] {
        let mut s = title(base, length);
        let before = s.project().clone();
        assert!(matches!(
            s.apply(apply()),
            Err(engine::EditError::Invalid(_))
        ));
        assert_eq!(s.project(), &before);
        assert_eq!(s.history_len(), (0, 0));
    }
    let mut s = title(None, 254_016_000_000);
    for params in [
        json!({"item":3,"preset":"loop"}),
        json!({"item":3,"preset":"fade_in","extra":0}),
        json!({"item":9007199254740992_u64,"preset":"fade_in"}),
        json!({"item":3}),
    ] {
        assert!(s.execute("text.apply_animation", params).is_err());
    }
    for id in [ItemId(999), ItemId(1), ItemId(4)] {
        assert!(
            s.apply(Command::ApplyTextAnimation {
                item: id,
                preset: FadeInOut
            })
            .is_err()
        );
    }
    for keys in [
        vec![(-1, 0.5)],
        vec![(254_016_000_001, 0.5)],
        vec![(0, 0.0), (76_204_800_000, 0.9)],
    ] {
        let param = Param::Value(
            Channel::new(
                ParamValue::Scalar(1.0),
                keys.into_iter()
                    .map(|(t, v)| Keyframe::new(Tick(t), ParamValue::Scalar(v)))
                    .collect(),
                Extrapolation::Hold,
                Extrapolation::Hold,
            )
            .unwrap(),
        );
        s.apply(Command::SetAnimatedParam {
            item: ItemId(3),
            path: path(),
            param,
        })
        .unwrap();
        let before = s.project().clone();
        let history = s.history_len();
        let revision = s.revision();
        assert!(s.apply(apply()).is_err());
        assert_eq!(s.project(), &before);
        assert_eq!((s.history_len(), s.revision()), (history, revision));
    }
    let mut s = title(None, 254_016_000_000);
    for (command, params) in [
        (
            "keyframes.set_extrapolation",
            json!({"item":3,"path":"opacity","component":null,"before":"linear","after":"hold"}),
        ),
        (
            "keyframes.set_handle",
            json!({"item":3,"path":"opacity","component":null,"time":0,"side":"out","handle":{"dt":1,"dv":0.1},"tangent":"broken"}),
        ),
    ] {
        s.apply(apply()).unwrap();
        s.execute(command, params).unwrap();
        let before = s.project().clone();
        assert!(s.apply(apply()).is_err());
        assert_eq!(s.project(), &before);
        s.undo().unwrap();
        s.undo().unwrap();
    }
    let mut s = title(None, 254_016_000_000);
    s.set_selection([ItemId(3)].into_iter().collect()).unwrap();
    let before = s.project().clone();
    assert!(
        s.apply(Command::Batch {
            commands: vec![
                apply(),
                Command::Delete {
                    items: vec![ItemId(999)]
                }
            ]
        })
        .is_err()
    );
    assert_eq!(s.project(), &before);
    assert_eq!(s.selection(), &[ItemId(3)].into_iter().collect());
    s.begin_gesture().unwrap();
    s.apply(apply()).unwrap();
    s.cancel_gesture().unwrap();
    assert_eq!(s.project(), &before);
    assert_eq!(s.history_len(), (0, 0));
    assert!(s.take_change().is_none());
    s.begin_gesture().unwrap();
    s.apply(apply()).unwrap();
    s.commit_gesture().unwrap();
    assert_eq!(s.history_len(), (1, 0));
    s.apply(Command::SetTrackLocked {
        track: TrackId(3),
        locked: true,
    })
    .unwrap();
    assert!(matches!(
        s.apply(apply()),
        Err(engine::EditError::Locked(TrackId(3)))
    ));
    let scene = s
        .apply(Command::CreateScene {
            name: "Other".into(),
        })
        .unwrap()
        .created[0];
    let engine::CreatedId::Scene(scene) = scene else {
        panic!("scene")
    };
    s.apply(Command::SwitchScene { scene }).unwrap();
    assert!(matches!(
        s.apply(apply()),
        Err(engine::EditError::NoItem(ItemId(3)))
    ));
}

#[test]
fn text_animation_keys_keep_ordinary_trim_split_move_and_scale_semantics() {
    let mut s = title(Some(ParamValue::Scalar(0.6)), 254_016_000_000);
    s.execute(
        "text.apply_animation",
        json!({"item":3,"preset":"fade_in_out"}),
    )
    .unwrap();
    let original = item(&s, ItemId(3)).clone();
    s.apply(Command::TrimStart {
        item: ItemId(3),
        at: frame(102),
    })
    .unwrap();
    near(
        value(&item(&s, ItemId(3)).animations, Tick::ZERO),
        0.6 * 16_951_334_400.0 / 76_204_800_000.0,
    );
    s.apply(Command::TrimEnd {
        item: ItemId(3),
        at: frame(125),
    })
    .unwrap();
    assert_eq!(
        item(&s, ItemId(3))
            .animations
            .get(&path())
            .unwrap()
            .channel(None)
            .unwrap()
            .keys()[3]
            .time,
        Tick(237_064_665_600)
    );
    let outcome = s
        .apply(Command::SplitAt {
            item: ItemId(3),
            at: frame(110),
        })
        .unwrap();
    let right = outcome
        .created
        .iter()
        .find_map(|id| {
            if let engine::CreatedId::Item(id) = id {
                Some(*id)
            } else {
                None
            }
        })
        .unwrap();
    let curve = item(&s, right).animations.clone();
    s.apply(Command::MoveItem {
        item: right,
        track: TrackId(3),
        start: frame(200),
    })
    .unwrap();
    assert_eq!(item(&s, right).animations, curve);
    near(
        value(&curve, Tick::ZERO),
        0.6 * 84_756_672_000.0 / 76_204_800_000.0_f64.max(84_756_672_000.0),
    );
    s.apply(Command::ScaleKeyframeTime {
        item: right,
        numerator: 2,
        denominator: 1,
    })
    .unwrap();
    for t in [0, 1, 38_102_400_000, 100_000_000_000] {
        near(
            value(&item(&s, right).animations, Tick(t * 2)),
            value(&curve, Tick(t)),
        );
    }
    for (numerator, denominator) in [(i64::MAX, 1), (1, 254_016_000_000)] {
        let before = s.project().clone();
        assert!(
            s.apply(Command::ScaleKeyframeTime {
                item: right,
                numerator,
                denominator
            })
            .is_err()
        );
        assert_eq!(s.project(), &before);
    }
    for params in [json!({"items":[right.0],"speed":2.0,"maintain_pitch":true})] {
        assert!(s.execute("timeline.set_speed", params).is_err());
    }
    while s.history_len().0 > 1 {
        s.undo().unwrap();
    }
    assert_eq!(item(&s, ItemId(3)), &original);
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

#[test]
fn clipboard_translates_output_local_bezier_snapshot_with_one_undo() {
    use engine::{KeyframePasteSource::Stored, KeyframeRef};
    let mut data = session().project().data().clone();
    data.assets.push(
        testkit::source::asset(99, std::env::temp_dir().join("vibecut-399-never-open.mp4"))
            .duration(frame(1000))
            .stream(
                testkit::source::video()
                    .duration(frame(1000))
                    .size(64, 64)
                    .build(),
            )
            .build(),
    );
    let video = data
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|t| &mut t.items)
        .find(|i| i.id == ItemId(2))
        .unwrap();
    video.source_asset = Some(media::AssetId(99));
    video.content = ItemParams::Video(VideoParams {
        source_audio_enabled: false,
        volume: 0.0,
        muted: false,
        crop: None,
        fades: Default::default(),
    });
    video.retime.speed = time::Speed::from_thousandths(2000).unwrap();
    let mut s = Session::new(Project::new(data).unwrap(), 100).unwrap();
    let q = frame(1).0;
    let mut first = Keyframe::new(Tick(-3 * q), ParamValue::Scalar(0.0));
    first.segment = Segment::Bezier;
    first.tangent = TangentMode::Broken;
    first.outgoing = Some(CurveHandle {
        dt: Tick(4 * q),
        dv: 0.0,
    });
    let mut last = Keyframe::new(Tick(9 * q), ParamValue::Scalar(1.0));
    last.tangent = TangentMode::Aligned;
    last.incoming = Some(CurveHandle {
        dt: Tick(-4 * q),
        dv: 0.0,
    });
    last.outgoing = Some(CurveHandle {
        dt: Tick(2 * q),
        dv: -0.1,
    });
    s.apply(Command::SetAnimatedParam {
        item: ItemId(2),
        path: path(),
        param: Param::Value(
            Channel::new(
                ParamValue::Scalar(0.7),
                vec![first.clone(), last.clone()],
                Extrapolation::Linear,
                Extrapolation::Hold,
            )
            .unwrap(),
        ),
    })
    .unwrap();
    s.take_change();
    let revision = s.revision();
    let history = s.history_len();
    let project = s.project().clone();
    let refs = [-3, 9]
        .map(|n| KeyframeRef {
            item: ItemId(2),
            path: path(),
            component: None,
            time: Tick(n * q),
        })
        .to_vec();
    assert!(
        !s.apply(Command::CopyKeyframes { keys: refs.clone() })
            .unwrap()
            .changed
    );
    assert_ne!(s.revision(), revision);
    assert!(s.take_change().is_none());
    assert_eq!(s.history_len(), history);
    assert_eq!(s.project(), &project);
    let copied_rev = s.revision();
    s.apply(Command::CopyKeyframes { keys: refs }).unwrap();
    s.apply(Command::CopyKeyframes { keys: vec![] }).unwrap();
    assert_eq!(s.revision(), copied_rev);
    // Source range begins at 50 frames and item starts at 100. Only output start
    // participates: paste at scene 98 yields -2 and +10 local frame ticks.
    s.apply(Command::SetAnimatedParam {
        item: ItemId(2),
        path: path(),
        param: Param::new(ParamValue::Scalar(0.2)).unwrap(),
    })
    .unwrap();
    s.apply(Command::TrimEnd {
        item: ItemId(2),
        at: frame(101),
    })
    .unwrap();
    assert_eq!(item(&s, ItemId(2)).duration().unwrap(), frame(1));
    let before = s.project().clone();
    let history = s.history_len();
    assert!(
        s.apply(Command::PasteKeyframes {
            item: ItemId(2),
            at: Tick(98 * q),
            source: Stored
        })
        .unwrap()
        .changed
    );
    assert_eq!(s.history_len().0, history.0 + 1);
    let pasted = s.project().clone();
    let channel = item(&s, ItemId(2))
        .animations
        .get(&path())
        .unwrap()
        .channel(None)
        .unwrap();
    first.time = Tick(-2 * q);
    last.time = Tick(10 * q);
    assert_eq!(channel.keys(), [first, last]);
    assert_eq!(channel.base(), &ParamValue::Scalar(0.7));
    assert_eq!(
        channel.extrapolation(),
        (Extrapolation::Linear, Extrapolation::Hold)
    );
    for (t, v) in [(-2, 0.0), (1, 0.15625), (4, 0.5), (7, 0.84375), (10, 1.0)] {
        near(value(&item(&s, ItemId(2)).animations, Tick(t * q)), v);
    }
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &before);
    s.apply(Command::Redo {}).unwrap();
    assert_eq!(s.project(), &pasted);
    let rev = s.revision();
    assert!(
        !s.apply(Command::PasteKeyframes {
            item: ItemId(2),
            at: Tick(98 * q),
            source: Stored
        })
        .unwrap()
        .changed
    );
    assert_eq!(s.revision(), rev);
    assert_eq!(s.history_len().0, history.0 + 1);
}

#[test]
fn clipboard_color_participation_held_values_and_equal_time_full_replacement() {
    use engine::{KeyframeClipboard, KeyframePasteSource::Payload};
    let mut s = session();
    let color_path: AnimationPath = BuiltinPath::Color.into();
    let mut target = Param::new(ParamValue::Color([0.6, 0.7, 0.8, 0.9])).unwrap();
    for component in [
        ColorComponent::R,
        ColorComponent::G,
        ColorComponent::B,
        ColorComponent::A,
    ] {
        let c = target.channel_mut(Some(component)).unwrap();
        let mut k = Keyframe::new(Tick(4), c.base().clone());
        k.outgoing = Some(CurveHandle {
            dt: Tick(1),
            dv: 0.03,
        });
        c.put(k).unwrap();
    }
    s.apply(Command::SetAnimatedParam {
        item: ItemId(2),
        path: color_path.clone(),
        param: target.clone(),
    })
    .unwrap();
    let mut source = Param::new(ParamValue::Color([0.2, 0.3, 0.4, 0.5])).unwrap();
    source
        .channel_mut(Some(ColorComponent::R))
        .unwrap()
        .put(Keyframe::new(Tick(-3), ParamValue::Scalar(0.8)))
        .unwrap();
    source
        .channel_mut(Some(ColorComponent::A))
        .unwrap()
        .put(Keyframe::new(Tick(1), ParamValue::Scalar(0.25)))
        .unwrap();
    let payload = KeyframeClipboard::new(Tick(-3), vec![(color_path.clone(), source)]).unwrap();
    s.apply(Command::PasteKeyframes {
        item: ItemId(2),
        at: frame(100),
        source: Payload { payload },
    })
    .unwrap();
    let param = item(&s, ItemId(2)).animations.get(&color_path).unwrap();
    for component in [ColorComponent::G, ColorComponent::B] {
        assert_eq!(
            param.channel(Some(component)).unwrap(),
            target.channel(Some(component)).unwrap()
        );
    }
    let r = param.channel(Some(ColorComponent::R)).unwrap();
    assert_eq!(r.base(), &ParamValue::Scalar(0.2));
    assert_eq!(r.keys().len(), 2);
    let a = param.channel(Some(ColorComponent::A)).unwrap();
    assert_eq!(a.base(), &ParamValue::Scalar(0.5));
    assert_eq!(a.keys(), [Keyframe::new(Tick(4), ParamValue::Scalar(0.25))]);
    for base in [
        ParamValue::Bool(false),
        ParamValue::Text("Tiếng Việt".into()),
    ] {
        let p = AnimationPath::new("params.held").unwrap();
        s.apply(Command::SetAnimatedParam {
            item: ItemId(2),
            path: p.clone(),
            param: Param::new(base.clone()).unwrap(),
        })
        .unwrap();
        let next = match base {
            ParamValue::Bool(_) => ParamValue::Bool(true),
            _ => ParamValue::Text("Đã dán".into()),
        };
        let mut k = Keyframe::new(Tick(2), next.clone());
        k.segment = Segment::Step;
        let payload = KeyframeClipboard::new(
            Tick(2),
            vec![(
                p.clone(),
                Param::Value(
                    Channel::new(
                        base.clone(),
                        vec![k],
                        Extrapolation::Hold,
                        Extrapolation::Hold,
                    )
                    .unwrap(),
                ),
            )],
        )
        .unwrap();
        s.apply(Command::PasteKeyframes {
            item: ItemId(2),
            at: frame(100),
            source: Payload { payload },
        })
        .unwrap();
        let param = item(&s, ItemId(2)).animations.get(&p).unwrap();
        assert_eq!(param.value_at(Tick(-1)).unwrap(), base);
        for t in [0, 1, 12] {
            assert_eq!(param.value_at(Tick(t)).unwrap(), next);
        }
        s.apply(Command::RemoveKeyframe {
            item: ItemId(2),
            path: p.clone(),
            component: None,
            time: Tick::ZERO,
        })
        .unwrap();
        assert_eq!(
            item(&s, ItemId(2))
                .animations
                .get(&p)
                .unwrap()
                .value_at(Tick(1))
                .unwrap(),
            base
        );
    }
}

#[test]
fn clipboard_refusal_is_atomic_across_paths_locks_batches_and_stale_refs() {
    use engine::{KeyframeClipboard, KeyframePasteSource, KeyframeRef};
    let mut s = session();
    let k = KeyframeRef {
        item: ItemId(2),
        path: path(),
        component: None,
        time: frame(0),
    };
    s.apply(Command::CopyKeyframes {
        keys: vec![k.clone()],
    })
    .unwrap();
    let before = s.project().clone();
    let rev = s.revision();
    let history = s.history_len();
    for keys in [
        vec![k.clone(), k.clone()],
        vec![KeyframeRef {
            time: frame(3),
            ..k.clone()
        }],
        vec![KeyframeRef {
            component: Some(ColorComponent::A),
            ..k.clone()
        }],
        vec![
            k.clone(),
            KeyframeRef {
                item: ItemId(99),
                ..k.clone()
            },
        ],
        vec![k.clone(); 4097],
    ] {
        assert!(s.apply(Command::CopyKeyframes { keys }).is_err());
        assert_eq!(s.project(), &before);
        assert_eq!(s.revision(), rev);
        assert_eq!(s.history_len(), history);
    }
    let param = Param::Value(
        Channel::new(
            ParamValue::Scalar(0.7),
            vec![Keyframe::new(Tick(0), ParamValue::Scalar(0.4))],
            Extrapolation::Hold,
            Extrapolation::Hold,
        )
        .unwrap(),
    );
    let payload = KeyframeClipboard::new(
        Tick(0),
        vec![
            (path(), param.clone()),
            (AnimationPath::new("params.absent").unwrap(), param),
        ],
    )
    .unwrap();
    let paste = Command::PasteKeyframes {
        item: ItemId(2),
        at: frame(100),
        source: KeyframePasteSource::Payload { payload },
    };
    assert!(s.apply(paste.clone()).is_err());
    assert!(
        s.apply(Command::Batch {
            commands: vec![
                Command::RenameProject {
                    name: "staged".into()
                },
                paste
            ]
        })
        .is_err()
    );
    for nonedit in [
        Command::CopyKeyframes { keys: vec![k] },
        Command::ReadClipboard {},
    ] {
        assert!(
            s.apply(Command::Batch {
                commands: vec![
                    Command::RenameProject {
                        name: "staged".into()
                    },
                    nonedit
                ]
            })
            .is_err()
        );
        assert_eq!(s.project(), &before);
        assert_eq!(s.revision(), rev);
        assert_eq!(s.history_len(), history);
    }
    s.apply(Command::SetTrackLocked {
        track: TrackId(2),
        locked: true,
    })
    .unwrap();
    let locked = s.project().clone();
    let locked_rev = s.revision();
    assert!(
        s.apply(Command::PasteKeyframes {
            item: ItemId(2),
            at: frame(100),
            source: KeyframePasteSource::Stored
        })
        .is_err()
    );
    assert_eq!(s.project(), &locked);
    assert_eq!(s.revision(), locked_rev);
    s.apply(Command::CopyKeyframes {
        keys: vec![KeyframeRef {
            item: ItemId(2),
            path: path(),
            component: None,
            time: frame(0),
        }],
    })
    .unwrap();
    s.apply(Command::SetTrackLocked {
        track: TrackId(2),
        locked: false,
    })
    .unwrap();
    let before = s.project().clone();
    let history = s.history_len().0;
    let paste = |at| Command::PasteKeyframes {
        item: ItemId(2),
        at,
        source: KeyframePasteSource::Stored,
    };
    s.apply(Command::Batch {
        commands: vec![paste(frame(101)), paste(frame(102))],
    })
    .unwrap();
    assert_eq!(s.history_len().0, history + 1);
    s.undo().unwrap();
    assert_eq!(s.project(), &before);
    let revision = s.revision();
    let history = s.history_len();
    assert!(
        s.apply(Command::Batch {
            commands: vec![
                paste(frame(103)),
                Command::SetTrackLocked {
                    track: TrackId(2),
                    locked: true
                },
                paste(frame(104))
            ]
        })
        .is_err()
    );
    assert_eq!(s.project(), &before);
    assert_eq!(s.revision(), revision);
    assert_eq!(s.history_len(), history);
    let mut fresh = Session::new((**s.project()).clone(), 100).unwrap();
    assert!(
        !fresh
            .apply(Command::PasteKeyframes {
                item: ItemId(999),
                at: Tick(1),
                source: KeyframePasteSource::Stored
            })
            .unwrap()
            .changed
    );
}

#[test]
fn clipboard_closed_payload_rejects_hostile_time_version_shape_and_color() {
    let mut s = session();
    let example = registry()
        .iter()
        .find(|r| r.id == "clipboard.paste_keyframes")
        .unwrap()
        .example
        .params
        .clone();
    let before = s.project().clone();
    let rev = s.revision();
    let history = s.history_len();
    for (pointer, value) in [
        ("/source/payload/version", json!(2)),
        ("/source/payload/origin", json!(1)),
        ("/source/payload/origin", json!(-9_007_199_254_740_992i64)),
        ("/source/payload/extra", json!(true)),
        ("/source/payload/params", json!([])),
        (
            "/source/payload/params/0/param/channels/keys/0/outgoing",
            json!({"dt":-1,"dv":0}),
        ),
        (
            "/source/payload/params/0/param/channels/value/value",
            json!(1e13),
        ),
    ] {
        let mut p = example.clone();
        if pointer.ends_with("/extra") {
            p["source"]["payload"]["extra"] = value;
        } else {
            *p.pointer_mut(pointer).unwrap() = value;
        }
        assert!(
            s.execute("clipboard.paste_keyframes", p).is_err(),
            "{pointer}"
        );
        assert_eq!(s.project(), &before);
        assert_eq!(s.revision(), rev);
        assert_eq!(s.history_len(), history);
    }
    use engine::KeyframeClipboard;
    let invalid = Param::Color(Box::new(std::array::from_fn(|_| {
        Channel::new(
            ParamValue::Scalar(1.1),
            vec![Keyframe::new(Tick(0), ParamValue::Scalar(0.5))],
            Extrapolation::Hold,
            Extrapolation::Hold,
        )
        .unwrap()
    })));
    assert!(KeyframeClipboard::new(Tick(0), vec![(BuiltinPath::Color.into(), invalid)]).is_err());
    let mut k = Keyframe::new(Tick(9_007_199_254_740_991), ParamValue::Scalar(0.5));
    k.outgoing = Some(CurveHandle {
        dt: Tick(1),
        dv: 0.0,
    });
    let p = Param::Value(
        Channel::new(
            ParamValue::Scalar(0.5),
            vec![Keyframe::new(Tick(-1), ParamValue::Scalar(0.0)), k],
            Extrapolation::Hold,
            Extrapolation::Hold,
        )
        .unwrap(),
    );
    assert!(KeyframeClipboard::new(Tick(-1), vec![(path(), p)]).is_err());
}

#[test]
fn clipboard_key_path_byte_and_batch_limits_preserve_the_previous_snapshot() {
    use engine::{KeyframeClipboard, KeyframePasteSource, KeyframeRef};
    let mut s = session();
    s.apply(Command::CopyKeyframes {
        keys: vec![KeyframeRef {
            item: ItemId(2),
            path: path(),
            component: None,
            time: Tick::ZERO,
        }],
    })
    .unwrap();
    let keys = (0..4096)
        .map(|n| Keyframe::new(Tick(n), ParamValue::Scalar(0.5)))
        .collect();
    let param = Param::Value(
        Channel::new(
            ParamValue::Scalar(0.7),
            keys,
            Extrapolation::Hold,
            Extrapolation::Hold,
        )
        .unwrap(),
    );
    let payload = KeyframeClipboard::new(Tick(0), vec![(path(), param)]).unwrap();
    s.apply(Command::PasteKeyframes {
        item: ItemId(2),
        at: frame(100),
        source: KeyframePasteSource::Payload { payload },
    })
    .unwrap_err(); // 4096 plus noncolliding old key exceeds the channel budget.
    let param = Param::Value(
        Channel::new(
            ParamValue::Scalar(0.7),
            vec![Keyframe::new(Tick(0), ParamValue::Scalar(0.5))],
            Extrapolation::Hold,
            Extrapolation::Hold,
        )
        .unwrap(),
    );
    let two = vec![
        (
            path(),
            Param::Value(
                Channel::new(
                    ParamValue::Scalar(0.7),
                    (0..4096)
                        .map(|n| Keyframe::new(Tick(n), ParamValue::Scalar(0.5)))
                        .collect(),
                    Extrapolation::Hold,
                    Extrapolation::Hold,
                )
                .unwrap(),
            ),
        ),
        (AnimationPath::new("params.more").unwrap(), param.clone()),
    ];
    assert!(matches!(
        KeyframeClipboard::new(Tick(0), two),
        Err(engine::EditError::Limit)
    ));
    let paths: Vec<_> = (0..64)
        .map(|n| {
            (
                AnimationPath::new(format!("params.p{n:02}")).unwrap(),
                param.clone(),
            )
        })
        .collect();
    assert!(KeyframeClipboard::new(Tick(0), paths.clone()).is_ok());
    let mut too_many = paths;
    too_many.push((AnimationPath::new("params.p64").unwrap(), param));
    assert!(matches!(
        KeyframeClipboard::new(Tick(0), too_many),
        Err(engine::EditError::Limit)
    ));
    let p = AnimationPath::new("params.text").unwrap();
    let text = |bytes| {
        let mut key = Keyframe::new(Tick(0), ParamValue::Text("a".repeat(bytes).into()));
        key.segment = Segment::Step;
        Param::Value(
            Channel::new(
                ParamValue::Text("".into()),
                vec![key],
                Extrapolation::Hold,
                Extrapolation::Hold,
            )
            .unwrap(),
        )
    };
    assert!(matches!(
        KeyframeClipboard::new(Tick(0), vec![(p.clone(), text(1 << 20))]),
        Err(engine::EditError::Limit)
    ));
    let mut boundary = json!({"version":1,"origin":0,"params":[{"path":"params.text","param":{"kind":"value","channels":{"value":{"kind":"text","value":""},"keys":[{"time":0,"value":{"kind":"text","value":""},"segment":"step","tangent":"flat","incoming":null,"outgoing":null}],"before":"hold","after":"hold"}}}]});
    let overhead = serde_json::to_vec(&boundary).unwrap().len();
    boundary["params"][0]["param"]["channels"]["keys"][0]["value"]["value"] =
        json!("a".repeat((1 << 20) - overhead));
    assert_eq!(serde_json::to_vec(&boundary).unwrap().len(), 1 << 20);
    let admitted: KeyframeClipboard = serde_json::from_value(boundary.clone()).unwrap();
    assert_eq!(serde_json::to_vec(&admitted).unwrap().len(), 1 << 20);
    let mut extra = boundary["params"][0]["param"]["channels"]["keys"][0]["value"]["value"]
        .as_str()
        .unwrap()
        .to_owned();
    extra.push('a');
    boundary["params"][0]["param"]["channels"]["keys"][0]["value"]["value"] = json!(extra);
    assert_eq!(serde_json::to_vec(&boundary).unwrap().len(), (1 << 20) + 1);
    assert!(serde_json::from_value::<KeyframeClipboard>(boundary).is_err());
    let payload = KeyframeClipboard::new(Tick(0), vec![(p, text(400_000))]).unwrap();
    let before = s.project().clone();
    let rev = s.revision();
    let history = s.history_len();
    let commands = (0..3)
        .map(|_| Command::PasteKeyframes {
            item: ItemId(2),
            at: Tick(1),
            source: KeyframePasteSource::Payload {
                payload: payload.clone(),
            },
        })
        .collect();
    assert!(matches!(
        s.apply(Command::Batch { commands }),
        Err(engine::EditError::Limit)
    ));
    assert_eq!(s.project(), &before);
    assert_eq!(s.revision(), rev);
    assert_eq!(s.history_len(), history);
    // The refused replacement did not lose the earlier captured scalar snapshot.
    s.apply(Command::SetAnimatedParam {
        item: ItemId(2),
        path: path(),
        param: Param::new(ParamValue::Scalar(0.3)).unwrap(),
    })
    .unwrap();
    s.apply(Command::PasteKeyframes {
        item: ItemId(2),
        at: frame(100),
        source: KeyframePasteSource::Stored,
    })
    .unwrap();
    assert_eq!(
        item(&s, ItemId(2))
            .animations
            .get(&path())
            .unwrap()
            .channel(None)
            .unwrap()
            .keys()[0]
            .value,
        ParamValue::Scalar(0.0)
    );
}

#[test]
fn clipboard_preserves_every_curve_label_and_flat_endpoint_excursions() {
    use engine::{KeyframePasteSource::Stored, KeyframeRef};
    for segment in [Segment::Step, Segment::Linear, Segment::Bezier] {
        for tangent in [
            TangentMode::Auto,
            TangentMode::Aligned,
            TangentMode::Broken,
            TangentMode::Flat,
        ] {
            for handles in [false, true] {
                let mut s = session();
                let mut a = Keyframe::new(Tick(0), ParamValue::Scalar(0.0));
                a.segment = segment;
                a.tangent = tangent;
                let mut b = Keyframe::new(Tick(12), ParamValue::Scalar(0.0));
                b.tangent = tangent;
                if handles {
                    a.outgoing = Some(CurveHandle {
                        dt: Tick(4),
                        dv: 1.0,
                    });
                    b.incoming = Some(CurveHandle {
                        dt: Tick(-4),
                        dv: 1.0,
                    });
                }
                let source = Param::Value(
                    Channel::new(
                        ParamValue::Scalar(0.7),
                        vec![a.clone(), b.clone()],
                        Extrapolation::Hold,
                        Extrapolation::Hold,
                    )
                    .unwrap(),
                );
                s.apply(Command::SetAnimatedParam {
                    item: ItemId(2),
                    path: path(),
                    param: source,
                })
                .unwrap();
                s.apply(Command::CopyKeyframes {
                    keys: [0, 12]
                        .map(|t| KeyframeRef {
                            item: ItemId(2),
                            path: path(),
                            component: None,
                            time: Tick(t),
                        })
                        .to_vec(),
                })
                .unwrap();
                s.apply(Command::SetAnimatedParam {
                    item: ItemId(2),
                    path: path(),
                    param: Param::new(ParamValue::Scalar(0.2)).unwrap(),
                })
                .unwrap();
                s.apply(Command::PasteKeyframes {
                    item: ItemId(2),
                    at: frame(100).checked_add(Tick(20)).unwrap(),
                    source: Stored,
                })
                .unwrap();
                a.time = Tick(20);
                b.time = Tick(32);
                let c = item(&s, ItemId(2))
                    .animations
                    .get(&path())
                    .unwrap()
                    .channel(None)
                    .unwrap();
                assert_eq!(c.keys(), [a, b]);
                for (t, v) in [(20, 0.0), (23, 0.5625), (26, 0.75), (29, 0.5625), (32, 0.0)] {
                    let expected = if segment == Segment::Bezier && handles {
                        v
                    } else {
                        0.0
                    };
                    let ParamValue::Scalar(actual) = c.value_at(Tick(t)) else {
                        panic!("scalar")
                    };
                    near(actual, expected);
                }
            }
        }
    }
}

#[test]
fn clipboard_final_project_budget_counts_replacement_instead_of_old_plus_new_keys() {
    use engine::{KeyframeClipboard, KeyframePasteSource::Payload};
    let mut data = session().project().data().clone();
    let target = data
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|t| &mut t.items)
        .find(|i| i.id == ItemId(2))
        .unwrap();
    target.animations = Animations::default();
    for n in 0..25 {
        let path = AnimationPath::new(format!("params.p{n:02}")).unwrap();
        let keys = (0..4000)
            .map(|k| Keyframe::new(Tick(k), ParamValue::Scalar(0.0)))
            .collect();
        target
            .animations
            .set(
                path,
                Param::Value(
                    Channel::new(
                        ParamValue::Scalar(0.0),
                        keys,
                        Extrapolation::Hold,
                        Extrapolation::Hold,
                    )
                    .unwrap(),
                ),
            )
            .unwrap();
    }
    let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let param = Param::Value(
        Channel::new(
            ParamValue::Scalar(0.7),
            vec![Keyframe::new(Tick(0), ParamValue::Scalar(0.5))],
            Extrapolation::Hold,
            Extrapolation::Hold,
        )
        .unwrap(),
    );
    let payload = KeyframeClipboard::new(
        Tick(0),
        vec![(AnimationPath::new("params.p00").unwrap(), param)],
    )
    .unwrap();
    s.apply(Command::PasteKeyframes {
        item: ItemId(2),
        at: frame(100),
        source: Payload {
            payload: payload.clone(),
        },
    })
    .unwrap();
    assert_eq!(item(&s, ItemId(2)).animations.key_count(), 100_000);
    let before = s.project().clone();
    let rev = s.revision();
    let history = s.history_len();
    assert!(
        s.apply(Command::PasteKeyframes {
            item: ItemId(2),
            at: frame(100).checked_add(Tick(5000)).unwrap(),
            source: Payload { payload }
        })
        .is_err()
    );
    assert_eq!(s.project(), &before);
    assert_eq!(s.revision(), rev);
    assert_eq!(s.history_len(), history);
}

#[test]
fn clipboard_copy_revision_distinguishes_signed_zero_snapshot_bytes() {
    use engine::{KeyframePasteSource::Stored, KeyframeRef};
    let mut s = session();
    s.apply(Command::SetAnimatedParam {
        item: ItemId(2),
        path: path(),
        param: Param::Value(
            Channel::new(
                ParamValue::Scalar(0.0),
                vec![
                    Keyframe::new(Tick::ZERO, ParamValue::Scalar(0.0)),
                    Keyframe::new(frame(120), ParamValue::Scalar(1.0)),
                ],
                Extrapolation::Linear,
                Extrapolation::Hold,
            )
            .unwrap(),
        ),
    })
    .unwrap();
    let copy = || Command::CopyKeyframes {
        keys: vec![KeyframeRef {
            item: ItemId(2),
            path: path(),
            component: None,
            time: Tick::ZERO,
        }],
    };
    s.apply(copy()).unwrap();
    // A real edit changes an unselected neighbor as well, so the committed
    // source contains -0.0 while the selected snapshot differs only in its bits.
    s.apply(Command::SetAnimatedParam {
        item: ItemId(2),
        path: path(),
        param: Param::Value(
            Channel::new(
                ParamValue::Scalar(0.0),
                vec![
                    Keyframe::new(Tick::ZERO, ParamValue::Scalar(-0.0)),
                    Keyframe::new(frame(120), ParamValue::Scalar(0.75)),
                ],
                Extrapolation::Linear,
                Extrapolation::Hold,
            )
            .unwrap(),
        ),
    })
    .unwrap();
    let before = s.revision();
    let history = s.history_len();
    assert!(!s.apply(copy()).unwrap().changed);
    assert_ne!(s.revision(), before);
    assert_eq!(s.history_len(), history);
    let copied = s.revision();
    s.apply(copy()).unwrap();
    assert_eq!(s.revision(), copied);
    s.apply(Command::SetAnimatedParam {
        item: ItemId(2),
        path: path(),
        param: Param::new(ParamValue::Scalar(0.7)).unwrap(),
    })
    .unwrap();
    s.apply(Command::PasteKeyframes {
        item: ItemId(2),
        at: frame(100),
        source: Stored,
    })
    .unwrap();
    let ParamValue::Scalar(v) = item(&s, ItemId(2))
        .animations
        .get(&path())
        .unwrap()
        .channel(None)
        .unwrap()
        .keys()[0]
        .value
    else {
        panic!("scalar")
    };
    assert_eq!(v.to_bits(), (-0.0_f64).to_bits());
}
