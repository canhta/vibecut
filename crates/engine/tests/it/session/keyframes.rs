#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "validated keyframe fixtures"
)]
use crate::support::session::Wire;
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

fn overlay(length: i64) -> Session {
    let mut s = session().project().data().clone();
    s.canvas.width = 64;
    s.canvas.height = 64;
    let target = s
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|t| &mut t.items)
        .find(|i| i.id == ItemId(2))
        .unwrap();
    target.animations = Animations::default();
    target.source_range =
        SourceRange::new(frame(50), frame(50).checked_add(Tick(length)).unwrap()).unwrap();
    Session::new(Project::new(s).unwrap(), 100).unwrap()
}

/// A text title (item 3) or an overlay clip (item 2), `frames` canvas frames long.
fn rig(overlay_clip: bool, frames: i64) -> (Session, ItemId) {
    if overlay_clip {
        (overlay(frame(frames).0), ItemId(2))
    } else {
        (title(None, frame(frames).0), ItemId(3))
    }
}

fn item_in(p: &Project, id: ItemId) -> &Item {
    p.data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == id)
        .unwrap()
}

type Slot = engine::AnimationSlot;
type Look = engine::AnimationPreset;
fn animate(
    s: &mut Session,
    id: ItemId,
    slot: Slot,
    preset: Option<Look>,
    duration: Option<Tick>,
) -> std::result::Result<engine::Outcome, engine::EditError> {
    s.apply(Command::SetClipAnimation {
        item: id,
        slot,
        preset,
        duration,
    })
}
fn read(s: &Session, id: ItemId) -> engine::ClipAnimations {
    engine::ClipAnimations::of(item(s, id), &s.project().data().canvas)
}
fn scalar(value: &ParamValue) -> f64 {
    let ParamValue::Scalar(value) = value else {
        panic!("scalar")
    };
    *value
}
fn keys_of(s: &Session, id: ItemId, path: BuiltinPath) -> Vec<(i64, f64)> {
    item(s, id)
        .animations
        .get(&path.into())
        .map(|param| {
            param
                .channel(None)
                .unwrap()
                .keys()
                .iter()
                .map(|key| (key.time.0, scalar(&key.value)))
                .collect()
        })
        .unwrap_or_default()
}
const OWNED: [BuiltinPath; 6] = [
    BuiltinPath::Opacity,
    BuiltinPath::TransformPositionX,
    BuiltinPath::TransformPositionY,
    BuiltinPath::TransformScaleX,
    BuiltinPath::TransformScaleY,
    BuiltinPath::TransformRotate,
];
fn rest(path: BuiltinPath) -> f64 {
    if matches!(
        path,
        BuiltinPath::Opacity | BuiltinPath::TransformScaleX | BuiltinPath::TransformScaleY
    ) {
        1.0
    } else {
        0.0
    }
}
fn only(s: &Session, id: ItemId, expected: &[BuiltinPath]) {
    for path in OWNED {
        assert_eq!(
            keys_of(s, id, path).is_empty(),
            !expected.contains(&path),
            "{path:?}"
        );
    }
}

#[test]
fn clip_animation_edge_presets_expand_keys_for_text_and_clips_and_undo_as_one_step() {
    use BuiltinPath::{
        Opacity, TransformPositionX as X, TransformPositionY as Y, TransformRotate as R,
        TransformScaleX as SX, TransformScaleY as SY,
    };
    // Canvas 64x64: a slide travels 16 px. Values are where an In look starts; the Out look
    // ends at the mirror (offsets negate, scale factors reflect around 1, opacity stays 0).
    let table: [(Look, &[(BuiltinPath, f64)]); 12] = [
        (Look::Fade, &[(Opacity, 0.0)]),
        (Look::SlideLeft, &[(X, 16.0)]),
        (Look::SlideRight, &[(X, -16.0)]),
        (Look::SlideUp, &[(Y, 16.0)]),
        (Look::SlideDown, &[(Y, -16.0)]),
        (Look::ZoomIn, &[(SX, 0.5), (SY, 0.5)]),
        (Look::ZoomOut, &[(SX, 1.5), (SY, 1.5)]),
        (Look::Spin, &[(Opacity, 0.0), (R, -180.0)]),
        (Look::Drop, &[(Y, -16.0), (Opacity, 0.0)]),
        (Look::Rise, &[(Y, 16.0), (Opacity, 0.0)]),
        (Look::Pop, &[(SX, 0.2), (SY, 0.2), (Opacity, 0.0)]),
        (Look::Tilt, &[(R, -30.0), (Opacity, 0.0)]),
    ];
    assert_eq!(table.len(), Look::EDGE.len());
    let length = frame(60).0;
    let half_second = 127_008_000_000_i64;
    for on_clip in [false, true] {
        for (preset, starts) in table {
            assert!(Look::EDGE.contains(&preset));
            for slot in [Slot::In, Slot::Out] {
                let (mut s, id) = rig(on_clip, 60);
                let before = s.project().clone();
                let content = item(&s, id).content.clone();
                assert!(
                    animate(&mut s, id, slot, Some(preset), None)
                        .unwrap()
                        .changed
                );
                assert_eq!(s.history_len(), (1, 0));
                only(
                    &s,
                    id,
                    &starts.iter().map(|(path, _)| *path).collect::<Vec<_>>(),
                );
                for (path, start) in starts {
                    let base = rest(*path);
                    let outer = match (slot, path) {
                        (Slot::In, _) => *start,
                        (_, Opacity) => *start,
                        (_, SX | SY) => 2.0 - *start,
                        _ => -*start,
                    };
                    let expected = if slot == Slot::In {
                        vec![(0, outer), (half_second, base)]
                    } else {
                        vec![(length - half_second, base), (length, outer)]
                    };
                    let got = keys_of(&s, id, *path);
                    assert_eq!(got.len(), 2, "{preset:?} {path:?}");
                    for ((gt, gv), (et, ev)) in got.into_iter().zip(expected) {
                        assert_eq!(gt, et, "{preset:?} {slot:?} {path:?}");
                        near(gv, ev);
                    }
                }
                assert_eq!(item(&s, id).content, content);
                let held = Some((preset, Tick(half_second)));
                assert_eq!(
                    read(&s, id),
                    engine::ClipAnimations {
                        enter: if slot == Slot::In { held } else { None },
                        exit: if slot == Slot::Out { held } else { None },
                        combo: None,
                    }
                );
                let saved = encode(s.project(), true).unwrap();
                assert_eq!(decode(&saved).unwrap().project, **s.project());
                s.undo().unwrap();
                assert_eq!(s.project(), &before);
                s.redo().unwrap();
                // An exact repeat changes nothing and records nothing.
                let revision = s.revision();
                assert!(
                    !animate(&mut s, id, slot, Some(preset), None)
                        .unwrap()
                        .changed
                );
                assert_eq!((s.revision(), s.history_len()), (revision, (1, 0)));
                // None removes the look in one step and leaves the item as it began.
                assert!(animate(&mut s, id, slot, None, None).unwrap().changed);
                assert_eq!(s.history_len(), (2, 0));
                assert_eq!(item(&s, id).animations, item_in(&before, id).animations);
            }
        }
    }
}

#[test]
fn clip_animation_combo_presets_span_the_whole_clip() {
    use BuiltinPath::{
        Opacity, TransformPositionX as X, TransformPositionY as Y, TransformRotate as R,
        TransformScaleX as SX, TransformScaleY as SY,
    };
    // Channel, key count, value at the last key; the first key is at 0 and the last at the end.
    let table: [(Look, &[(BuiltinPath, usize, f64)]); 13] = [
        (Look::Pulse, &[(SX, 3, 1.0), (SY, 3, 1.0)]),
        (Look::Heartbeat, &[(SX, 8, 1.0), (SY, 8, 1.0)]),
        (Look::Sway, &[(R, 4, 0.0)]),
        (Look::Swing, &[(R, 3, -10.0)]),
        (Look::Float, &[(Y, 3, 0.0)]),
        (Look::DriftLeft, &[(X, 2, -3.2)]),
        (Look::DriftRight, &[(X, 2, 3.2)]),
        (Look::PushIn, &[(SX, 2, 1.15), (SY, 2, 1.15)]),
        (Look::PullOut, &[(SX, 2, 1.0), (SY, 2, 1.0)]),
        (Look::Shake, &[(X, 9, 0.0)]),
        (Look::Flicker, &[(Opacity, 7, 1.0)]),
        (Look::Breathe, &[(SX, 3, 1.0), (SY, 3, 1.0)]),
        (Look::Turn, &[(R, 2, 360.0)]),
    ];
    assert_eq!(table.len(), Look::COMBO.len());
    let length = frame(60).0;
    for on_clip in [false, true] {
        for (preset, channels) in table {
            let (mut s, id) = rig(on_clip, 60);
            let before = s.project().clone();
            assert!(
                animate(&mut s, id, Slot::Combo, Some(preset), None)
                    .unwrap()
                    .changed
            );
            only(
                &s,
                id,
                &channels
                    .iter()
                    .map(|(path, _, _)| *path)
                    .collect::<Vec<_>>(),
            );
            for (path, count, last) in channels {
                let got = keys_of(&s, id, *path);
                assert_eq!(got.len(), *count, "{preset:?} {path:?}");
                assert_eq!(got[0].0, 0);
                assert_eq!(got[got.len() - 1].0, length);
                assert!(got.windows(2).all(|pair| pair[0].0 < pair[1].0));
                near(got[got.len() - 1].1, *last);
            }
            assert_eq!(
                read(&s, id),
                engine::ClipAnimations {
                    combo: Some(preset),
                    ..Default::default()
                }
            );
            let saved = encode(s.project(), true).unwrap();
            assert_eq!(decode(&saved).unwrap().project, **s.project());
            s.undo().unwrap();
            assert_eq!(s.project(), &before);
            s.redo().unwrap();
            assert!(
                animate(&mut s, id, Slot::Combo, None, None)
                    .unwrap()
                    .changed
            );
            assert_eq!(item(&s, id).animations, item_in(&before, id).animations);
        }
    }
    // Combo excludes In and Out in both directions.
    let (mut s, id) = rig(true, 60);
    animate(&mut s, id, Slot::In, Some(Look::Fade), None).unwrap();
    animate(&mut s, id, Slot::Combo, Some(Look::Pulse), None).unwrap();
    assert_eq!(read(&s, id).enter, None);
    assert!(keys_of(&s, id, BuiltinPath::Opacity).is_empty());
    animate(&mut s, id, Slot::Out, Some(Look::Fade), None).unwrap();
    assert_eq!(read(&s, id).combo, None);
    assert!(keys_of(&s, id, BuiltinPath::TransformScaleX).is_empty());
}

#[test]
fn clip_animation_durations_clamp_to_the_clip_and_the_later_edge_shrinks() {
    let f = |n: i64| frame(n);
    let length = f(60);
    // (In frames, Out frames) requested in that order, then what each ends as.
    let rows: [((i64, i64), (i64, i64)); 6] = [
        ((20, 20), (20, 20)),
        ((40, 40), (40, 20)),
        ((50, 30), (50, 10)),
        ((1000, 30), (59, 1)),
        ((0, 0), (1, 1)),
        ((0, 7), (1, 7)),
    ];
    for on_clip in [false, true] {
        for ((want_in, want_out), (got_in, got_out)) in rows {
            let (mut s, id) = rig(on_clip, 60);
            animate(&mut s, id, Slot::In, Some(Look::Fade), Some(f(want_in))).unwrap();
            animate(&mut s, id, Slot::Out, Some(Look::Fade), Some(f(want_out))).unwrap();
            let now = read(&s, id);
            assert_eq!(
                now.enter,
                Some((Look::Fade, f(got_in))),
                "{want_in} {want_out}"
            );
            assert_eq!(
                now.exit,
                Some((Look::Fade, f(got_out))),
                "{want_in} {want_out}"
            );
            assert!(f(got_in).0 + f(got_out).0 <= length.0);
        }
    }
    // A later, longer In shrinks the Out already there; Out never grows back by itself.
    let (mut s, id) = rig(false, 60);
    animate(&mut s, id, Slot::Out, Some(Look::Fade), Some(f(30))).unwrap();
    animate(&mut s, id, Slot::In, Some(Look::Fade), Some(f(45))).unwrap();
    assert_eq!(read(&s, id).exit, Some((Look::Fade, f(15))));
    animate(&mut s, id, Slot::In, Some(Look::Fade), Some(f(10))).unwrap();
    assert_eq!(read(&s, id).exit, Some((Look::Fade, f(15))));
    // Changing only the duration keeps the look; a lone In may span the whole clip.
    let (mut s, id) = rig(false, 60);
    animate(&mut s, id, Slot::In, Some(Look::SlideUp), None).unwrap();
    animate(&mut s, id, Slot::In, Some(Look::SlideUp), Some(f(900))).unwrap();
    assert_eq!(read(&s, id).enter, Some((Look::SlideUp, length)));
    // Mixed looks share the opacity channel only where they both use it.
    let (mut s, id) = rig(false, 60);
    animate(&mut s, id, Slot::In, Some(Look::Fade), Some(f(10))).unwrap();
    animate(&mut s, id, Slot::Out, Some(Look::Pop), Some(f(10))).unwrap();
    assert_eq!(
        keys_of(&s, id, BuiltinPath::Opacity),
        [(0, 0.0), (f(10).0, 1.0), (f(50).0, 1.0), (f(60).0, 0.0)]
    );
    assert_eq!(keys_of(&s, id, BuiltinPath::TransformScaleX).len(), 2);
    animate(&mut s, id, Slot::In, None, None).unwrap();
    assert_eq!(
        keys_of(&s, id, BuiltinPath::Opacity),
        [(f(50).0, 1.0), (f(60).0, 0.0)]
    );
    assert_eq!(read(&s, id).exit, Some((Look::Pop, f(10))));
}

#[test]
fn trimming_a_clip_refits_its_animations_in_one_step() {
    let f = |n: i64| frame(n);
    for on_clip in [false, true] {
        for edge in ["end", "start"] {
            let (mut s, id) = rig(on_clip, 60);
            let start = item(&s, id).start;
            animate(&mut s, id, Slot::In, Some(Look::Fade), Some(f(20))).unwrap();
            animate(&mut s, id, Slot::Out, Some(Look::Fade), Some(f(20))).unwrap();
            let animated = s.project().clone();
            let history = s.history_len().0;
            let command = if edge == "end" {
                Command::TrimEnd {
                    item: id,
                    at: start.checked_add(f(30)).unwrap(),
                }
            } else {
                Command::TrimStart {
                    item: id,
                    at: start.checked_add(f(30)).unwrap(),
                }
            };
            s.apply(command).unwrap();
            assert_eq!(s.history_len().0, history + 1);
            assert_eq!(item(&s, id).duration().unwrap(), f(30));
            // Out shrinks to the 10 frames left after In; the two meet at one resting key.
            assert_eq!(
                read(&s, id),
                engine::ClipAnimations {
                    enter: Some((Look::Fade, f(20))),
                    exit: Some((Look::Fade, f(10))),
                    combo: None,
                }
            );
            assert_eq!(
                keys_of(&s, id, BuiltinPath::Opacity),
                [(0, 0.0), (f(20).0, 1.0), (f(30).0, 0.0)]
            );
            s.undo().unwrap();
            assert_eq!(s.project(), &animated);
            // A trim to a single frame leaves no room for an animation and drops it.
            s.apply(Command::TrimEnd {
                item: id,
                at: start.checked_add(f(1)).unwrap(),
            })
            .unwrap();
            assert_eq!(read(&s, id), engine::ClipAnimations::default());
            assert!(keys_of(&s, id, BuiltinPath::Opacity).is_empty());
        }
    }
    // A Combo stretches over the new length; keys the user changed by hand are left alone.
    let (mut s, id) = rig(true, 60);
    let start = item(&s, id).start;
    animate(&mut s, id, Slot::Combo, Some(Look::Pulse), None).unwrap();
    s.apply(Command::TrimEnd {
        item: id,
        at: start.checked_add(f(20)).unwrap(),
    })
    .unwrap();
    let scale = keys_of(&s, id, BuiltinPath::TransformScaleX);
    assert_eq!(
        scale.iter().map(|k| k.0).collect::<Vec<_>>(),
        [0, f(10).0, f(20).0]
    );
    assert_eq!(read(&s, id).combo, Some(Look::Pulse));
    let (mut s, id) = rig(true, 60);
    let start = item(&s, id).start;
    animate(&mut s, id, Slot::In, Some(Look::Fade), Some(f(20))).unwrap();
    s.apply(Command::DeleteKeyframes {
        keys: vec![engine::KeyframeRef {
            item: id,
            path: path(),
            component: None,
            time: f(20),
        }],
    })
    .unwrap();
    assert_eq!(read(&s, id), engine::ClipAnimations::default());
    let edited = keys_of(&s, id, BuiltinPath::Opacity);
    s.apply(Command::TrimEnd {
        item: id,
        at: start.checked_add(f(30)).unwrap(),
    })
    .unwrap();
    assert_eq!(keys_of(&s, id, BuiltinPath::Opacity), edited);
}

#[test]
fn clip_animation_refusals_and_transactions_preserve_all_state() {
    let apply = || Command::SetClipAnimation {
        item: ItemId(3),
        slot: Slot::In,
        preset: Some(Look::Fade),
        duration: None,
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
        json!({"item":3,"slot":"in","preset":"loop"}),
        json!({"item":3,"slot":"middle","preset":"fade"}),
        json!({"item":3,"slot":"in","preset":"fade","extra":0}),
        json!({"item":9007199254740992_u64,"slot":"in","preset":"fade"}),
        json!({"item":3,"preset":"fade"}),
        json!({"item":3,"slot":"in","preset":"fade","duration":"long"}),
    ] {
        assert!(s.wire("clip.set_animation", params).is_err());
    }
    // Wire form: a duration in ticks, and null removes.
    s.wire(
        "clip.set_animation",
        json!({"item":3,"slot":"out","preset":"slide_left","duration":127008000000_i64}),
    )
    .unwrap();
    assert_eq!(
        read(&s, ItemId(3)).exit,
        Some((Look::SlideLeft, Tick(127_008_000_000)))
    );
    s.wire(
        "clip.set_animation",
        json!({"item":3,"slot":"out","preset":null,"duration":null}),
    )
    .unwrap();
    assert_eq!(read(&s, ItemId(3)), engine::ClipAnimations::default());
    s.undo().unwrap();
    s.undo().unwrap();
    for id in [ItemId(999), ItemId(1), ItemId(4)] {
        assert!(
            s.apply(Command::SetClipAnimation {
                item: id,
                slot: Slot::In,
                preset: Some(Look::Fade),
                duration: None,
            })
            .is_err()
        );
    }
    // A look belongs to its slot; a duration needs an In or Out look.
    for (slot, preset, duration) in [
        (Slot::In, Some(Look::Pulse), None),
        (Slot::Out, Some(Look::Turn), None),
        (Slot::Combo, Some(Look::Fade), None),
        (Slot::Combo, Some(Look::Pulse), Some(Tick(1))),
        (Slot::Combo, None, Some(Tick(1))),
        (Slot::In, None, Some(Tick(1))),
    ] {
        let before = s.project().clone();
        assert!(matches!(
            s.apply(Command::SetClipAnimation {
                item: ItemId(3),
                slot,
                preset,
                duration,
            }),
            Err(engine::EditError::Invalid(_))
        ));
        assert_eq!(s.project(), &before);
    }
    // Keys the animations did not write block a change on their channel only.
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
        assert!(
            !animate(&mut s, ItemId(3), Slot::Out, None, None)
                .unwrap()
                .changed,
            "an empty slot changes nothing"
        );
        assert_eq!(s.project(), &before);
        assert_eq!((s.history_len(), s.revision()), (history, revision));
        animate(&mut s, ItemId(3), Slot::In, Some(Look::SlideLeft), None).unwrap();
        animate(&mut s, ItemId(3), Slot::In, None, None).unwrap();
        s.undo().unwrap();
        s.undo().unwrap();
        s.undo().unwrap();
    }
    // Hand-edited keys read as no animation and block a new look.
    let mut s = title(None, 254_016_000_000);
    animate(&mut s, ItemId(3), Slot::In, Some(Look::Fade), None).unwrap();
    s.apply(Command::DeleteKeyframes {
        keys: vec![engine::KeyframeRef {
            item: ItemId(3),
            path: path(),
            component: None,
            time: Tick::ZERO,
        }],
    })
    .unwrap();
    assert_eq!(read(&s, ItemId(3)), engine::ClipAnimations::default());
    let before = s.project().clone();
    assert!(animate(&mut s, ItemId(3), Slot::In, Some(Look::Fade), None).is_err());
    assert_eq!(s.project(), &before);
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
        let mut s = title(None, 254_016_000_000);
        s.apply(apply()).unwrap();
        s.wire(command, params).unwrap();
        // Curve edits do not change what the keys say, so the look is still found.
        assert_eq!(read(&s, ItemId(3)).enter.map(|e| e.0), Some(Look::Fade));
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
fn clip_animation_keys_keep_ordinary_split_move_and_scale_semantics() {
    let mut s = title(Some(ParamValue::Scalar(0.6)), 254_016_000_000);
    animate(&mut s, ItemId(3), Slot::In, Some(Look::Fade), None).unwrap();
    animate(&mut s, ItemId(3), Slot::Out, Some(Look::Fade), None).unwrap();
    let original = item(&s, ItemId(3)).clone();
    near(value(&original.animations, Tick(63_504_000_000)), 0.3);
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
    assert!(
        s.wire(
            "timeline.set_speed",
            json!({"items":[right.0],"speed":2.0,"maintain_pitch":true})
        )
        .is_err()
    );
    while s.history_len().0 > 2 {
        s.undo().unwrap();
    }
    assert_eq!(item(&s, ItemId(3)), &original);
}

#[test]
fn text_stroke_colour_keys_are_ordinary_item_relative_colour_channels() {
    use project::ColorComponent;
    let mut s = title(None, 254_016_000_000);
    let stroke = AnimationPath::from(BuiltinPath::StrokeColor);
    let from = [1.0, 0.0, 0.0, 1.0];
    let to = [0.0, 0.0, 1.0, 1.0];
    let channels: [Channel; 4] = std::array::from_fn(|i| {
        Channel::new(
            ParamValue::Scalar(from[i]),
            vec![
                Keyframe::new(Tick::ZERO, ParamValue::Scalar(from[i])),
                Keyframe::new(frame(20), ParamValue::Scalar(to[i])),
            ],
            Extrapolation::Hold,
            Extrapolation::Hold,
        )
        .unwrap()
    });
    s.apply(Command::SetAnimatedParam {
        item: ItemId(3),
        path: stroke.clone(),
        param: Param::Color(Box::new(channels)),
    })
    .unwrap();
    assert_eq!(s.history_len(), (1, 0));
    let colour = |s: &Session, id: ItemId| match item(s, id)
        .animations
        .value_at(&stroke, Tick::ZERO)
        .unwrap()
    {
        Some(ParamValue::Color(value)) => value,
        other => panic!("{other:?}"),
    };
    assert_eq!(colour(&s, ItemId(3)), from);
    s.apply(Command::TrimStart {
        item: ItemId(3),
        at: frame(102),
    })
    .unwrap();
    let keys = item(&s, ItemId(3))
        .animations
        .get(&stroke)
        .unwrap()
        .channel(Some(ColorComponent::R))
        .unwrap()
        .keys()
        .to_vec();
    assert_eq!(keys[1].time, frame(18));
    let trimmed = colour(&s, ItemId(3));
    assert!(trimmed[0] < from[0] && trimmed[2] > from[2], "{trimmed:?}");
    // Split, move and scale preserve the ordinary colour curve.
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
    assert!(curve.get(&stroke).is_some());
    s.apply(Command::MoveItem {
        item: right,
        track: TrackId(3),
        start: frame(200),
    })
    .unwrap();
    assert_eq!(item(&s, right).animations, curve);
    s.apply(Command::ScaleKeyframeTime {
        item: right,
        numerator: 2,
        denominator: 1,
    })
    .unwrap();
    let scaled = item(&s, right)
        .animations
        .value_at(&stroke, frame(2))
        .unwrap();
    assert_eq!(scaled, curve.value_at(&stroke, frame(1)).unwrap());
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
    // Easing: one curve over two selected segments is one undo step; the last
    // key has no segment and refuses the whole selection untouched.
    let at = |n: i64| engine::KeyframeRef {
        item: ItemId(2),
        path: path(),
        component: None,
        time: frame(n),
    };
    let steps = s.history_len().0;
    assert!(
        s.apply(Command::ApplyEasingPreset {
            keys: vec![at(0), at(120)],
            preset: EasingPreset::Pop,
        })
        .is_err()
    );
    assert_eq!((s.project(), s.history_len().0), (&changed, steps));
    s.apply(Command::ApplyEasingPreset {
        keys: vec![at(0), at(80)],
        preset: EasingPreset::Smooth,
    })
    .unwrap();
    assert_eq!(s.history_len().0, steps + 1);
    let channel = item(&s, ItemId(2))
        .animations
        .get(&path())
        .unwrap()
        .channel(None)
        .unwrap()
        .clone();
    for time in [0, 80] {
        // Handles round to whole ticks, so compare within a thousandth of the span.
        let got = channel.segment_curve(frame(time)).unwrap();
        assert!(
            got.0
                .iter()
                .zip(EasingPreset::Smooth.curve().0)
                .all(|(a, b)| (a - b).abs() < 1e-3)
        );
    }
    s.apply(Command::SetKeyframeCurve {
        keys: vec![at(80)],
        curve: EasingCurve::LINEAR,
    })
    .unwrap();
    let linear = item(&s, ItemId(2))
        .animations
        .get(&path())
        .unwrap()
        .channel(None)
        .unwrap()
        .keys()[1]
        .clone();
    assert_eq!((linear.segment, linear.outgoing), (Segment::Linear, None));
    s.undo().unwrap();
    s.undo().unwrap();
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
        fit: Default::default(),
        cleanup: Default::default(),
        background: project::ClipBackground::None,
        reduce_noise: false,
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
    // Cut shares Copy's admission: an invalid target refuses before any delete or
    // clipboard change, and a valid cut is one undo step that removes the keys.
    for keys in [
        vec![k.clone(), k.clone()],
        vec![KeyframeRef {
            time: frame(3),
            ..k.clone()
        }],
    ] {
        assert!(
            s.apply(Command::CutSelection {
                target: engine::CutTarget::Keyframes { keys },
            })
            .is_err()
        );
        assert_eq!(s.project(), &before);
        assert_eq!(s.history_len(), history);
    }
    let mut cut = session();
    let original = cut.project().clone();
    let cut_history = cut.history_len();
    cut.apply(Command::CutSelection {
        target: engine::CutTarget::Keyframes {
            keys: vec![k.clone()],
        },
    })
    .unwrap();
    assert_ne!(cut.project(), &original);
    assert_eq!(cut.history_len().0, cut_history.0 + 1);
    assert!(cut.clipboard_has_entry());
    assert!(!cut.can_copy_keyframes(&[k.clone()]));
    cut.undo().unwrap();
    assert_eq!(cut.project(), &original);
    assert!(cut.clipboard_has_entry(), "undo keeps the cut payload");
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
        assert!(s.wire("clipboard.paste_keyframes", p).is_err(), "{pointer}");
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
        Err(engine::EditError::Control(
            engine::control::ControlError::Limit
        ))
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
        Err(engine::EditError::Control(
            engine::control::ControlError::Limit
        ))
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
        Err(engine::EditError::Control(
            engine::control::ControlError::Limit
        ))
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
        Err(engine::EditError::Control(
            engine::control::ControlError::Limit
        ))
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

#[test]
fn clipboard_copy_admission_counts_escaped_bases_and_selected_keys_at_the_byte_boundary() {
    use crate::support::control::{ok, run, runner};
    use engine::{EditError, KeyframeRef};
    // Independent wire fixture includes JSON escaping and UTF-8 in both base and key.
    let base = "\"\\\n\u{0001}Việt".repeat(20_000);
    let mut expected = json!({"version":1,"origin":0,"params":[{"path":"params.text","param":{"kind":"value","channels":{"value":{"kind":"text","value":base},"keys":[{"time":0,"value":{"kind":"text","value":"\"\\\nViệt"},"segment":"step","tangent":"flat","incoming":null,"outgoing":null}],"before":"hold","after":"hold"}}}]});
    let overhead = serde_json::to_vec(&expected).unwrap().len();
    let text = format!("\"\\\nViệt{}", "a".repeat((1 << 20) - overhead));
    expected["params"][0]["param"]["channels"]["keys"][0]["value"]["value"] = json!(text);
    assert_eq!(serde_json::to_vec(&expected).unwrap().len(), 1 << 20);
    for extra in [0, 1] {
        let mut data = session().project().data().clone();
        let mut selected = Keyframe::new(
            Tick(0),
            ParamValue::Text(format!("{text}{}", "b".repeat(extra)).into()),
        );
        selected.segment = Segment::Step;
        let mut unselected = Keyframe::new(Tick(1), ParamValue::Text("z".repeat(1 << 20).into()));
        unselected.segment = Segment::Step;
        data.scene_mut()
            .timeline
            .iter_mut()
            .flat_map(|t| &mut t.items)
            .next()
            .unwrap()
            .animations
            .set(
                AnimationPath::new("params.text").unwrap(),
                Param::Value(
                    Channel::new(
                        ParamValue::Text(base.clone().into()),
                        vec![selected, unselected],
                        Extrapolation::Hold,
                        Extrapolation::Hold,
                    )
                    .unwrap(),
                ),
            )
            .unwrap();
        let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
        let old = KeyframeRef {
            item: ItemId(2),
            path: path(),
            component: None,
            time: Tick::ZERO,
        };
        s.apply(Command::CopyKeyframes { keys: vec![old] }).unwrap();
        let mut r = runner();
        let stored =
            serde_json::to_value(ok(run(&mut s, &mut r, "clipboard.read", json!({}), "old")))
                .unwrap();
        let before = s.project().clone();
        let history = s.history_len();
        let rev = s.revision();
        let selection = s.selection().clone();
        let keys = vec![KeyframeRef {
            item: ItemId(2),
            path: AnimationPath::new("params.text").unwrap(),
            component: None,
            time: Tick::ZERO,
        }];
        assert_eq!(s.can_copy_keyframes(&keys), extra == 0);
        assert_eq!(s.revision(), rev);
        // Exercise strict wire Copy as well as typed admission.
        let result = s.wire("clipboard.copy_keyframes", json!({"keys":keys}));
        if extra == 0 {
            assert!(!result.unwrap().changed);
            let read =
                serde_json::to_value(ok(run(&mut s, &mut r, "clipboard.read", json!({}), "new")))
                    .unwrap();
            assert_eq!(read["payload"], expected);
            assert_eq!(serde_json::to_vec(&read["payload"]).unwrap().len(), 1 << 20);
        } else {
            assert!(matches!(
                result,
                Err(EditError::Control(engine::control::ControlError::Limit))
            ));
            assert_eq!(s.revision(), rev);
            let read = serde_json::to_value(ok(run(
                &mut s,
                &mut r,
                "clipboard.read",
                json!({}),
                "unchanged",
            )))
            .unwrap();
            assert_eq!(read, stored);
        }
        assert_eq!(s.project(), &before);
        assert_eq!(s.history_len(), history);
        assert_eq!(s.selection(), &selection);
    }
}

#[test]
fn clipboard_hostile_text_availability_and_copy_refuse_before_materializing_values() {
    use engine::{EditError, KeyframeRef};
    // A fresh subprocess isolates hostile admission from parallel tests and earlier heaps.
    // This is a real typed-valid 64 MiB source; the old capture cloned it before refusal.
    const CHILD: &str = "VIBECUT_CLIPBOARD_ALLOCATION_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "session::keyframes::clipboard_hostile_text_availability_and_copy_refuse_before_materializing_values", "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .output().unwrap();
        println!("{}", String::from_utf8_lossy(&output.stdout));
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        return;
    }
    let mut data = session().project().data().clone();
    let mut keys = Vec::new();
    for n in 0..128 {
        let mut key = Keyframe::new(Tick(n), ParamValue::Text("x".repeat(512 << 10).into()));
        key.segment = Segment::Step;
        keys.push(key);
    }
    let p = AnimationPath::new("params.text").unwrap();
    data.scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|t| &mut t.items)
        .next()
        .unwrap()
        .animations
        .set(
            p.clone(),
            Param::Value(
                Channel::new(
                    ParamValue::Text("base".into()),
                    keys,
                    Extrapolation::Hold,
                    Extrapolation::Hold,
                )
                .unwrap(),
            ),
        )
        .unwrap();
    let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
    s.apply(Command::CopyKeyframes {
        keys: vec![KeyframeRef {
            item: ItemId(2),
            path: path(),
            component: None,
            time: Tick::ZERO,
        }],
    })
    .unwrap();
    let before = s.project().clone();
    let rev = s.revision();
    let history = s.history_len();
    let selection = s.selection().clone();
    let refs: Vec<_> = (0..128)
        .map(|n| KeyframeRef {
            item: ItemId(2),
            path: p.clone(),
            component: None,
            time: Tick(n),
        })
        .collect();
    for _ in 0..8 {
        assert!(!s.can_copy_keyframes(&refs));
    }
    assert!(matches!(
        s.apply(Command::CopyKeyframes { keys: refs }),
        Err(EditError::Control(engine::control::ControlError::Limit))
    ));
    assert_eq!(s.project(), &before);
    assert_eq!(s.revision(), rev);
    assert_eq!(s.history_len(), history);
    assert_eq!(s.selection(), &selection);
    // The same real 64 MiB Text is unrelated to the small stored opacity snapshot.
    // Availability must inspect only that snapshot's paths; actual edits keep ordinary
    // atomic candidate ownership. No heap instrumentation is used as an oracle.
    for _ in 0..8 {
        assert!(s.can_paste_keyframes(ItemId(2), frame(100)));
        assert!(s.can_paste_keyframes(ItemId(2), frame(101)));
        assert!(!s.can_paste_keyframes(ItemId(2), Tick(-1)));
        assert!(!s.can_paste_keyframes(ItemId(999), frame(100)));
    }
    assert!(std::sync::Arc::ptr_eq(s.project(), &before));
    assert_eq!(s.revision(), rev);
    assert_eq!(s.history_len(), history);
    assert_eq!(s.selection(), &selection);
    assert!(s.take_change().is_none());
    let paste = |at| Command::PasteKeyframes {
        item: ItemId(2),
        at,
        source: engine::KeyframePasteSource::Stored,
    };
    assert!(!s.apply(paste(frame(100))).unwrap().changed);
    assert_eq!(s.revision(), rev);
    assert_eq!(s.history_len(), history);
    assert!(s.apply(paste(frame(101))).unwrap().changed);
    let dest = item(&s, ItemId(2));
    assert_eq!(
        dest.animations.get(&p),
        before
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|t| &t.items)
            .find(|i| i.id == ItemId(2))
            .unwrap()
            .animations
            .get(&p)
    );
    assert_eq!(
        dest.animations
            .get(&path())
            .unwrap()
            .channel(None)
            .unwrap()
            .keys()[1]
            .time,
        frame(1)
    );
    s.undo().unwrap();
    assert_eq!(s.project(), &before);
    let undone = s.revision();
    let redo_history = s.history_len();
    assert!(s.can_paste_keyframes(ItemId(2), frame(100)));
    assert!(!s.apply(paste(frame(100))).unwrap().changed);
    assert_eq!(s.revision(), undone);
    assert_eq!(s.history_len(), redo_history);
    assert_eq!(s.selection(), &selection);
    // Read through the consumed control boundary, proving the earlier scalar store survived.
    let mut r = crate::support::control::runner();
    let stored = crate::support::control::ok(crate::support::control::run(
        &mut s,
        &mut r,
        "clipboard.read",
        json!({}),
        "preserved",
    ));
    assert_eq!(
        serde_json::to_value(stored).unwrap()["payload"]["params"][0]["path"],
        "opacity"
    );
}

#[test]
fn clipboard_paste_eligibility_matches_locks_types_time_and_merged_channel_refusals() {
    use engine::{EditError, KeyframePasteSource::Stored, KeyframeRef};
    const SAFE: i64 = 9_007_199_254_740_991;
    for case in [
        "zero",
        "replacement",
        "full-replacement",
        "new-at-limit",
        "one-collision",
        "missing-path",
        "value-kind",
        "param-kind",
        "locked-repeat",
        "negative-scene",
        "unsafe-scene",
        "translated-overflow",
        "negative-local",
        "unsafe-local",
        "missing-item",
    ] {
        let p = AnimationPath::new("params.transfer").unwrap();
        let source_param = Param::Value(
            Channel::new(
                ParamValue::Scalar(0.7),
                vec![
                    Keyframe::new(Tick(0), ParamValue::Scalar(0.5)),
                    Keyframe::new(Tick(12), ParamValue::Scalar(0.9)),
                ],
                Extrapolation::Linear,
                Extrapolation::Hold,
            )
            .unwrap(),
        );
        let mut data = session().project().data().clone();
        let track = data
            .scene_mut()
            .timeline
            .iter_mut()
            .find(|t| t.id == TrackId(2))
            .unwrap();
        let target = &mut track.items[0];
        target.start = match case {
            "negative-local" => Tick(100),
            "unsafe-local" => Tick(SAFE + 1),
            _ => Tick::ZERO,
        };
        let param = match case {
            "missing-path" => None,
            "value-kind" => Some(Param::new(ParamValue::Text("held".into())).unwrap()),
            "param-kind" => Some(Param::new(ParamValue::Color([0.2; 4])).unwrap()),
            "replacement" | "locked-repeat" => Some(source_param.clone()),
            "full-replacement" | "new-at-limit" | "one-collision" => Some(Param::Value(
                Channel::new(
                    ParamValue::Scalar(0.1),
                    (0..4096)
                        .map(|t| Keyframe::new(Tick(t), ParamValue::Scalar(0.2)))
                        .collect(),
                    Extrapolation::Hold,
                    Extrapolation::Hold,
                )
                .unwrap(),
            )),
            _ => Some(Param::new(ParamValue::Scalar(0.1)).unwrap()),
        };
        if let Some(param) = param {
            target.animations.set(p.clone(), param).unwrap();
        }
        let mut source = target.clone();
        source.id = ItemId(3);
        source.start = target.start.checked_add(frame(800)).unwrap();
        source.animations = Animations::default();
        source.animations.set(p.clone(), source_param).unwrap();
        track.items.push(source);
        track.locked = case == "locked-repeat";
        let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
        s.apply(Command::CopyKeyframes {
            keys: [0, 12]
                .map(|t| KeyframeRef {
                    item: ItemId(3),
                    path: p.clone(),
                    component: None,
                    time: Tick(t),
                })
                .to_vec(),
        })
        .unwrap(); // Copy remains allowed on a locked source track.
        s.set_selection([ItemId(2)].into_iter().collect()).unwrap();
        let target_id = if case == "missing-item" {
            ItemId(999)
        } else {
            ItemId(2)
        };
        let at = match case {
            "negative-scene" => Tick(-1),
            "unsafe-scene" => Tick(SAFE + 1),
            "translated-overflow" => Tick(SAFE - 5),
            "new-at-limit" => Tick(4096),
            "one-collision" => Tick(4090),
            _ => Tick::ZERO,
        };
        let expected = matches!(
            case,
            "zero" | "replacement" | "full-replacement" | "negative-local"
        );
        let before = s.project().clone();
        let rev = s.revision();
        let history = s.history_len();
        let selection = s.selection().clone();
        for _ in 0..4 {
            assert_eq!(s.can_paste_keyframes(target_id, at), expected, "{case}");
        }
        assert!(std::sync::Arc::ptr_eq(s.project(), &before), "{case}");
        assert_eq!(s.revision(), rev, "{case}");
        assert_eq!(s.history_len(), history, "{case}");
        assert_eq!(s.selection(), &selection, "{case}");
        assert!(s.take_change().is_none(), "{case}");
        let result = s.apply(Command::PasteKeyframes {
            item: target_id,
            at,
            source: Stored,
        });
        assert_eq!(result.is_ok(), expected, "{case}: {result:?}");
        if expected {
            let changed = result.unwrap().changed;
            assert_eq!(changed, case != "replacement", "{case}");
            if changed {
                s.undo().unwrap();
            }
            assert_eq!(s.project(), &before, "{case}");
        } else {
            let error = result.unwrap_err();
            match case {
                "new-at-limit" | "one-collision" => assert!(
                    matches!(
                        error,
                        EditError::Project(project::Error::TooLarge("channel keys"))
                    ),
                    "{error:?}"
                ),
                "locked-repeat" => assert!(matches!(error, EditError::Locked(TrackId(2)))),
                "missing-path" => assert!(matches!(
                    error,
                    EditError::Project(project::Error::Invalid("missing animated parameter"))
                )),
                "missing-item" => assert!(matches!(error, EditError::NoItem(ItemId(999)))),
                "value-kind" => {
                    assert!(matches!(error, EditError::Invalid("clipboard value type")))
                }
                "param-kind" => assert!(matches!(
                    error,
                    EditError::Invalid("clipboard parameter type")
                )),
                _ => assert!(
                    matches!(
                        error,
                        EditError::Control(engine::control::ControlError::InvalidParams)
                    ),
                    "{case}: {error:?}"
                ),
            }
            assert_eq!(s.project(), &before, "{case}");
            assert_eq!(s.revision(), rev, "{case}");
            assert_eq!(s.history_len(), history, "{case}");
            assert_eq!(s.selection(), &selection, "{case}");
            assert!(s.take_change().is_none(), "{case}");
        }
    }
}

#[test]
fn clipboard_paste_eligibility_counts_only_participating_rgba_channels() {
    use engine::{KeyframePasteSource::Stored, KeyframeRef};
    for full_component in [ColorComponent::G, ColorComponent::R] {
        let p = AnimationPath::new("params.transferColor").unwrap();
        let mut data = session().project().data().clone();
        let track = data
            .scene_mut()
            .timeline
            .iter_mut()
            .find(|t| t.id == TrackId(2))
            .unwrap();
        let mut target = Param::new(ParamValue::Color([0.2; 4])).unwrap();
        *target.channel_mut(Some(full_component)).unwrap() = Channel::new(
            ParamValue::Scalar(0.2),
            (0..4096)
                .map(|t| Keyframe::new(Tick(t), ParamValue::Scalar(0.3)))
                .collect(),
            Extrapolation::Hold,
            Extrapolation::Hold,
        )
        .unwrap();
        track.items[0].animations.set(p.clone(), target).unwrap();
        let mut source = track.items[0].clone();
        source.id = ItemId(3);
        source.start = frame(800);
        let mut param = Param::new(ParamValue::Color([0.7; 4])).unwrap();
        param
            .channel_mut(Some(ColorComponent::R))
            .unwrap()
            .put(Keyframe::new(Tick::ZERO, ParamValue::Scalar(0.8)))
            .unwrap();
        source.animations.set(p.clone(), param).unwrap();
        track.items.push(source);
        let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
        s.apply(Command::CopyKeyframes {
            keys: vec![KeyframeRef {
                item: ItemId(3),
                path: p.clone(),
                component: Some(ColorComponent::R),
                time: Tick::ZERO,
            }],
        })
        .unwrap();
        let before = s.project().clone();
        let at = frame(100).checked_add(Tick(4096)).unwrap();
        let expected = full_component == ColorComponent::G;
        assert_eq!(s.can_paste_keyframes(ItemId(2), at), expected);
        assert!(std::sync::Arc::ptr_eq(s.project(), &before));
        let result = s.apply(Command::PasteKeyframes {
            item: ItemId(2),
            at,
            source: Stored,
        });
        assert_eq!(result.is_ok(), expected);
        if expected {
            let dest = item(&s, ItemId(2)).animations.get(&p).unwrap();
            let original = before
                .data()
                .scene()
                .timeline
                .iter()
                .flat_map(|t| &t.items)
                .find(|i| i.id == ItemId(2))
                .unwrap()
                .animations
                .get(&p)
                .unwrap();
            for component in [ColorComponent::G, ColorComponent::B, ColorComponent::A] {
                assert_eq!(
                    dest.channel(Some(component)).unwrap(),
                    original.channel(Some(component)).unwrap()
                );
            }
            assert_eq!(
                dest.channel(Some(ColorComponent::R)).unwrap().keys()[0].time,
                Tick(4096)
            );
        } else {
            assert_eq!(s.project(), &before);
        }
    }
}

#[test]
fn graphic_appearance_keys_follow_trim_split_move_and_time_scaling_without_media_speed() {
    let mut data = Project::empty(ProjectId(397), "Graphic keys".into())
        .unwrap()
        .into_data();
    let path = AnimationPath::new("params.strokeWidth").unwrap();
    let mut animations = Animations::default();
    animations
        .set(
            path.clone(),
            Param::Value(
                Channel::new(
                    ParamValue::Scalar(2.),
                    vec![
                        Keyframe::new(Tick(0), ParamValue::Scalar(0.)),
                        Keyframe::new(Tick(80), ParamValue::Scalar(64.)),
                    ],
                    Extrapolation::Hold,
                    Extrapolation::Hold,
                )
                .unwrap(),
            ),
        )
        .unwrap();
    data.scene_mut().timeline.main.items.push(Item {
        hidden: false,
        blend_mode: Default::default(),
        masks: vec![],
        effects: vec![],
        animations,
        id: ItemId(397),
        source_asset: None,
        source_range: SourceRange::new(Tick(30), Tick(130)).unwrap(),
        start: Tick(20),
        retime: Default::default(),
        content: ItemParams::Graphic(GraphicParams::rectangle()),
    });
    let mut s = Session::new(Project::new(data).unwrap(), 100).unwrap();
    s.apply(Command::TrimStart {
        item: ItemId(397),
        at: Tick(30),
    })
    .unwrap();
    let times = |s: &Session, id| {
        item(s, id)
            .animations
            .get(&path)
            .unwrap()
            .channel(None)
            .unwrap()
            .keys()
            .iter()
            .map(|k| k.time.0)
            .collect::<Vec<_>>()
    };
    assert_eq!(times(&s, ItemId(397)), vec![-10, 70]);
    let out = s
        .apply(Command::SplitAt {
            item: ItemId(397),
            at: Tick(60),
        })
        .unwrap();
    let right = out
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
    assert_eq!(times(&s, right), vec![-40, 40]);
    s.apply(Command::ScaleKeyframeTime {
        item: right,
        numerator: 2,
        denominator: 1,
    })
    .unwrap();
    assert_eq!(times(&s, right), vec![-80, 80]);
    s.apply(Command::TimelineMoveItems {
        items: vec![right],
        delta: Tick(10),
        placement: None,
    })
    .unwrap();
    assert_eq!(times(&s, right), vec![-80, 80]);
    let copied = s
        .apply(Command::DuplicateItems { items: vec![right] })
        .unwrap();
    let duplicate = copied
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
    assert_eq!(times(&s, duplicate), vec![-80, 80]);
    assert_eq!(
        item(&s, duplicate)
            .animations
            .value_at(&path, Tick(0))
            .unwrap(),
        Some(ParamValue::Scalar(32.))
    );
}
