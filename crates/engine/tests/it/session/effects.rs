#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use engine::{Command, Session};
use project::*;
use serde_json::json;
use time::{FrameRate, SourceRange, Tick};
fn session() -> Session {
    let mut data = Project::empty(ProjectId(1), "Effects".into())
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
                "effects.update_param",
                json!({"item":2,"effect":"blur1","key":"intensity","value":bad})
            )
            .is_err()
        );
        assert_eq!(s.project(), &added);
    }
    assert!(s.execute("effects.add", json!({"item":2,"effect":{"id":"blur1","enabled":true,"params":{"kind":"blur","intensity":15.0}}})).is_err());
    s.execute(
        "effects.update_param",
        json!({"item":2,"effect":"blur1","key":"intensity","value":35.0}),
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
        .scene()
        .timeline
        .overlays
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

fn item(s: &Session) -> &Item {
    &s.project().data().scene().timeline.overlays[0].items[0]
}
#[test]
fn stack_gestures_round_trip_and_refuse_invalid_orders() {
    let mut s = session();
    for id in ["a", "b"] {
        s.execute("effects.add", json!({"item":2,"effect":{"id":id,"enabled":true,"params":{"kind":"blur","intensity":15.0}}})).unwrap();
    }
    for (command, params) in [
        ("effects.reorder", json!({"item":2,"order":["b","a"]})),
        (
            "effects.toggle",
            json!({"item":2,"effect":"b","enabled":false}),
        ),
        (
            "effects.update_param",
            json!({"item":2,"effect":"a","key":"intensity","value":42.0}),
        ),
    ] {
        let before = s.project().clone();
        s.execute(command, params).unwrap();
        let after = s.project().clone();
        s.undo().unwrap();
        assert_eq!(s.project(), &before);
        s.redo().unwrap();
        assert_eq!(s.project(), &after);
    }
    let saved = project::encode(s.project(), false).unwrap();
    assert_eq!(project::decode(&saved).unwrap().project, **s.project());
    for order in [
        json!([]),
        json!(["a"]),
        json!(["a", "a"]),
        json!(["a", "missing"]),
        json!(["a", "b", "c"]),
    ] {
        assert!(
            s.execute("effects.reorder", json!({"item":2,"order":order}))
                .is_err()
        );
        assert_eq!(project::encode(s.project(), false).unwrap(), saved);
    }
    assert!(
        s.execute(
            "effects.update_param",
            json!({"item":2,"effect":"a","key":"unknown","value":1})
        )
        .is_err()
    );
    s.begin_gesture().unwrap();
    for value in [3.0, 99.0, 20.0] {
        s.execute(
            "effects.update_param",
            json!({"item":2,"effect":"a","key":"intensity","value":value}),
        )
        .unwrap();
    }
    s.commit_gesture().unwrap();
    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), saved);
    s.begin_gesture().unwrap();
    s.execute(
        "effects.toggle",
        json!({"item":2,"effect":"a","enabled":false}),
    )
    .unwrap();
    s.cancel_gesture().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), saved);
    s.apply(Command::SetTrackFlags {
        track: TrackId(1),
        muted: false,
        hidden: false,
        locked: true,
    })
    .unwrap();
    for (command, params) in [
        (
            "effects.add",
            json!({"item":2,"effect":{"id":"c","enabled":true,"params":{"kind":"blur","intensity":15}}}),
        ),
        ("effects.reorder", json!({"item":2,"order":["a","b"]})),
        (
            "effects.toggle",
            json!({"item":2,"effect":"a","enabled":false}),
        ),
        ("effects.remove", json!({"item":2,"effect":"a"})),
        (
            "effects.update_param",
            json!({"item":2,"effect":"a","key":"intensity","value":5}),
        ),
    ] {
        assert!(
            !engine::registry()
                .iter()
                .find(|spec| spec.id == command)
                .unwrap()
                .enabled(&s)
        );
        assert!(s.execute(command, params).is_err());
    }
}

#[test]
fn effect_keyframes_follow_trim_and_keyframe_time_scaling() {
    let mut s = session();
    s.execute("effects.add",json!({"item":2,"effect":{"id":"a","enabled":true,"params":{"kind":"blur","intensity":15.0}}})).unwrap();
    let path = AnimationPath::new("effects.a.params.intensity").unwrap();
    s.apply(Command::SetAnimatedParam {
        item: ItemId(2),
        path: path.clone(),
        param: Param::new(ParamValue::Scalar(0.0)).unwrap(),
    })
    .unwrap();
    let frame = FrameRate::FPS_30.tick_of(1).unwrap();
    for (n, value) in [(0, 0.0), (20, 100.0)] {
        s.apply(Command::AddKeyframe {
            item: ItemId(2),
            path: path.clone(),
            component: None,
            key: Keyframe::new(Tick(frame.0 * n), ParamValue::Scalar(value)),
        })
        .unwrap();
    }
    let value = |s: &Session, n| {
        item(s)
            .animations
            .value_at(&path, Tick(frame.0 * n))
            .unwrap()
    };
    assert_eq!(value(&s, 10), Some(ParamValue::Scalar(50.0)));
    s.apply(Command::TrimStart {
        item: ItemId(2),
        at: Tick(frame.0 * 5),
    })
    .unwrap();
    assert_eq!(value(&s, 5), Some(ParamValue::Scalar(50.0)));
    s.apply(Command::ScaleKeyframeTime {
        item: ItemId(2),
        numerator: 2,
        denominator: 1,
    })
    .unwrap();
    assert_eq!(value(&s, 10), Some(ParamValue::Scalar(50.0)));
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(value(&s, 10), Some(ParamValue::Scalar(50.0)));
}

struct Hooks;
struct Lease;
impl render::Reservation for Lease {}
impl render::ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn reserve(
        &self,
        _: render::MemoryKind,
        _: u64,
    ) -> render::Result<Box<dyn render::Reservation>> {
        Ok(Box::new(Lease))
    }
}
struct CountingEffects(std::sync::atomic::AtomicUsize);
impl render::effects::EffectCompositor for CountingEffects {
    fn apply(
        &self,
        image: &render::Image,
        passes: &[render::effects::EffectPass],
        inputs: render::effects::EffectInputs<'_>,
        hooks: &dyn render::ExecutionHooks,
    ) -> render::Result<render::Image> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        render::effects::CpuEffects.apply(image, passes, inputs, hooks)
    }
}
fn pump(r: &mut engine::jobs::JobRunner, id: engine::jobs::JobId) -> engine::jobs::JobSnapshot {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        r.tick().unwrap();
        let state = r.snapshot(id).unwrap();
        if matches!(
            state.state,
            engine::jobs::JobState::Completed
                | engine::jobs::JobState::Cancelled
                | engine::jobs::JobState::Failed
        ) {
            return state;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
}
#[test]
fn thumbnails_cache_by_content_evict_and_cancel_before_publication() {
    use engine::effect_thumbnails::*;
    use engine::jobs::*;
    use std::sync::{Arc, atomic::Ordering};
    let budget = engine::ByteBudget::new(64 << 20).unwrap();
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 4,
            retained: 4,
            network_attempts: 1,
            backoff: std::time::Duration::from_millis(10),
            progress_interval: std::time::Duration::from_millis(10),
        },
        budget.clone(),
        Arc::new(SystemClock::default()),
    )
    .unwrap();
    let adapter = Arc::new(CountingEffects(std::sync::atomic::AtomicUsize::new(0)));
    let cache = EffectThumbnails::new(adapter.clone(), budget.clone());
    let source = Arc::new(render::Image::new(32, 18, [0.5, 0.0, 0.0, 1.0], &Hooks).unwrap());
    for n in 0..2 {
        let (id, _) = cache
            .submit(&mut runner, source.clone(), EffectParams::default())
            .unwrap();
        assert_eq!(pump(&mut runner, id).state, JobState::Completed);
        assert_eq!(runner.take_output(id).unwrap().bytes().len(), 32 * 18 * 4);
        assert_eq!(adapter.0.load(Ordering::SeqCst), 1, "hit {n}");
    }
    let (id, token) = cache
        .submit(
            &mut runner,
            source.clone(),
            EffectParams::Blur { intensity: 99.0 },
        )
        .unwrap();
    token.cancel();
    assert_eq!(pump(&mut runner, id).state, JobState::Cancelled);
    assert!(runner.take_output(id).is_none());
    assert_eq!(adapter.0.load(Ordering::SeqCst), 1);
    for n in 0..MAX_THUMBNAILS {
        let (id, _) = cache
            .submit(
                &mut runner,
                source.clone(),
                EffectParams::Blur {
                    intensity: n as f64 + 30.0,
                },
            )
            .unwrap();
        assert_eq!(pump(&mut runner, id).state, JobState::Completed);
        runner.take_output(id).unwrap();
    }
    assert_eq!(
        budget.snapshot().unwrap().used,
        MAX_THUMBNAILS as u64 * 32 * 18 * 4
    );
    let (id, _) = cache
        .submit(&mut runner, source.clone(), EffectParams::default())
        .unwrap();
    assert_eq!(pump(&mut runner, id).state, JobState::Completed);
    runner.take_output(id).unwrap();
    assert_eq!(adapter.0.load(Ordering::SeqCst), MAX_THUMBNAILS + 2);
    assert!(
        cache
            .submit(
                &mut runner,
                source,
                EffectParams::Blur {
                    intensity: f64::NAN
                }
            )
            .is_err()
    );
    let large = Arc::new(render::Image::new(321, 1, [0.0; 4], &Hooks).unwrap());
    assert!(
        cache
            .submit(&mut runner, large, EffectParams::default())
            .is_err()
    );
}

struct InterruptedEffects {
    started: std::sync::mpsc::SyncSender<()>,
    fail: bool,
}
impl render::effects::EffectCompositor for InterruptedEffects {
    fn apply(
        &self,
        _: &render::Image,
        _: &[render::effects::EffectPass],
        _: render::effects::EffectInputs<'_>,
        hooks: &dyn render::ExecutionHooks,
    ) -> render::Result<render::Image> {
        self.started.send(()).unwrap();
        if self.fail {
            return Err(render::Error::Unsupported("test device"));
        }
        while !hooks.cancelled() {
            std::thread::yield_now();
        }
        Err(render::Error::Cancelled)
    }
}
#[test]
fn running_thumbnail_cancel_and_device_failure_keep_cache_empty() {
    use engine::{effect_thumbnails::*, jobs::*};
    use std::sync::Arc;
    for fail in [false, true] {
        let budget = engine::ByteBudget::new(64 << 20).unwrap();
        let mut runner = JobRunner::new(
            RunnerConfig {
                concurrency: [1, 1, 1],
                queue: 4,
                retained: 4,
                network_attempts: 1,
                backoff: std::time::Duration::from_millis(10),
                progress_interval: std::time::Duration::from_millis(10),
            },
            budget.clone(),
            Arc::new(SystemClock::default()),
        )
        .unwrap();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let cache = EffectThumbnails::new(
            Arc::new(InterruptedEffects { started: tx, fail }),
            budget.clone(),
        );
        let source = Arc::new(render::Image::new(32, 18, [0.0; 4], &Hooks).unwrap());
        let (id, token) = cache
            .submit(&mut runner, source, EffectParams::default())
            .unwrap();
        runner.tick().unwrap();
        rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        if !fail {
            token.cancel();
        }
        let result = pump(&mut runner, id);
        if fail {
            assert_eq!(result.state, JobState::Failed);
            assert!(matches!(
                result.error.as_deref(),
                Some(engine::EditError::Render(render::Error::Unsupported(_)))
            ));
        } else {
            assert_eq!(result.state, JobState::Cancelled);
        }
        assert!(runner.take_output(id).is_none());
        assert_eq!(budget.snapshot().unwrap().used, 0);
    }
}

#[test]
fn audio_refuses_even_an_empty_effect_reorder() {
    let mut data = session().project().data().clone();
    let mut track = data.scene_mut().timeline.overlays.remove(0);
    track.kind = TrackKind::Audio;
    let item = &mut track.items[0];
    item.content = ItemParams::Audio(AudioParams {
        fades: Default::default(),
        volume: 0.,
        muted: false,
        source: Default::default(),
    });
    item.source_asset = Some(media::AssetId(1));
    let duration = item.source_range.end();
    data.scene_mut().timeline.insert_track(track);
    data.assets.push(
        testkit::source::asset(1, std::env::temp_dir().join("audio-fixture.wav"))
            .format("wav")
            .duration(duration)
            .stream(
                testkit::source::audio()
                    .codec("pcm_s16le")
                    .time_base(media::TimeBase::new(1, 48000).unwrap())
                    .duration(duration)
                    .start(Tick::ZERO)
                    .channels(1)
                    .build(),
            )
            .build(),
    );
    let mut s = Session::new(Project::new(data).unwrap(), 20).unwrap();
    for (command, params) in [
        (
            "effects.add",
            json!({"item":2,"effect":{"id":"a","enabled":true,"params":{"kind":"blur","intensity":15}}}),
        ),
        ("effects.reorder", json!({"item":2,"order":[]})),
        (
            "effects.toggle",
            json!({"item":2,"effect":"a","enabled":false}),
        ),
        ("effects.remove", json!({"item":2,"effect":"a"})),
        (
            "effects.update_param",
            json!({"item":2,"effect":"a","key":"intensity","value":5}),
        ),
    ] {
        assert!(
            !engine::registry()
                .iter()
                .find(|spec| spec.id == command)
                .unwrap()
                .enabled(&s)
        );
        assert!(matches!(
            s.execute(command, params),
            Err(engine::EditError::Unsupported(_))
        ));
    }
}

#[test]
fn color_grade_controls_clamp_undo_per_gesture_and_persist() {
    let mut s = session();
    let neutral = json!({"id":"color_grade","enabled":true,"params":{"kind":"color_grade","brightness":0.0,"contrast":0.0,"saturation":0.0,"hue":0.0}});
    s.execute("effects.add", json!({"item":2,"effect":neutral}))
        .unwrap();
    let added = s.project().clone();
    // Every control: both limits accepted, one step past either refused atomically.
    for key in ["brightness", "contrast", "saturation", "hue"] {
        for edge in [-50.0, 50.0] {
            s.execute(
                "effects.update_param",
                json!({"item":2,"effect":"color_grade","key":key,"value":edge}),
            )
            .unwrap();
            s.apply(Command::Undo {}).unwrap();
            assert_eq!(s.project(), &added, "{key} {edge}");
        }
        let held = s.project().clone();
        for bad in [-50.5, 50.5, 1e300] {
            assert!(
                s.execute(
                    "effects.update_param",
                    json!({"item":2,"effect":"color_grade","key":key,"value":bad})
                )
                .is_err()
            );
            assert_eq!(s.project(), &held);
        }
    }
    assert!(
        s.execute(
            "effects.update_param",
            json!({"item":2,"effect":"color_grade","key":"intensity","value":1.0})
        )
        .is_err()
    );
    // A drag gesture of many updates is a single undo step.
    s.begin_gesture().unwrap();
    for v in [5.0, 12.0, 20.0] {
        s.execute(
            "effects.update_param",
            json!({"item":2,"effect":"color_grade","key":"contrast","value":v}),
        )
        .unwrap();
    }
    s.commit_gesture().unwrap();
    let graded = s.project().clone();
    assert!(matches!(
        item(&s).effects[0].params,
        EffectParams::ColorGrade { contrast, .. } if contrast == 20.0
    ));
    let saved = project::encode(&graded, false).unwrap();
    assert_eq!(&project::decode(&saved).unwrap().project, graded.as_ref());
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &added);
    s.apply(Command::Redo {}).unwrap();
    assert_eq!(s.project(), &graded);
}

#[test]
fn chroma_key_controls_clamp_undo_per_gesture_and_persist() {
    let mut s = session();
    let defaults = json!({"id":"chroma_key","enabled":true,"params":{"kind":"chroma_key","key_color":[0,255,0,255],"intensity":40.0,"shadow":0.0}});
    s.execute("effects.add", json!({"item":2,"effect":defaults}))
        .unwrap();
    let added = s.project().clone();
    // Both controls accept their limits and refuse one step beyond, atomically.
    for (key, edge) in [
        ("intensity", 0.0),
        ("intensity", 100.0),
        ("shadow", 1.0),
        ("shadow", 100.0),
    ] {
        s.execute(
            "effects.update_param",
            json!({"item":2,"effect":"chroma_key","key":key,"value":edge}),
        )
        .unwrap();
        s.apply(Command::Undo {}).unwrap();
        assert_eq!(s.project(), &added, "{key} {edge}");
    }
    for (key, bad) in [
        ("intensity", -0.5),
        ("intensity", 100.5),
        ("shadow", -0.5),
        ("shadow", 100.5),
        ("shadow", f64::NAN),
    ] {
        assert!(
            s.execute(
                "effects.update_param",
                json!({"item":2,"effect":"chroma_key","key":key,"value":bad})
            )
            .is_err(),
            "{key} {bad}"
        );
        assert_eq!(s.project(), &added);
    }
    // The picked colour is one packed sRGB scalar, so a pick is one command.
    s.execute(
        "effects.update_param",
        json!({"item":2,"effect":"chroma_key","key":"key_color","value":0x11_22_33 as f64}),
    )
    .unwrap();
    assert!(matches!(
        item(&s).effects[0].params,
        EffectParams::ChromaKey { key_color, .. } if key_color == Color([0x11, 0x22, 0x33, 255])
    ));
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &added);
    for bad in [-1.0, 16_777_216.0, 1.5, f64::NAN] {
        assert!(
            s.execute(
                "effects.update_param",
                json!({"item":2,"effect":"chroma_key","key":"key_color","value":bad})
            )
            .is_err(),
            "{bad}"
        );
        assert_eq!(s.project(), &added);
    }
    assert!(
        s.execute(
            "effects.update_param",
            json!({"item":2,"effect":"chroma_key","key":"contrast","value":1.0})
        )
        .is_err()
    );
    // A pick-and-drag gesture of many updates is a single undo step.
    s.begin_gesture().unwrap();
    for v in [5.0, 20.0, 30.0] {
        s.execute(
            "effects.update_param",
            json!({"item":2,"effect":"chroma_key","key":"intensity","value":v}),
        )
        .unwrap();
    }
    s.commit_gesture().unwrap();
    let keyed = s.project().clone();
    assert!(matches!(
        item(&s).effects[0].params,
        EffectParams::ChromaKey { intensity, .. } if intensity == 30.0
    ));
    let saved = project::encode(&keyed, false).unwrap();
    assert_eq!(&project::decode(&saved).unwrap().project, keyed.as_ref());
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &added);
    s.apply(Command::Redo {}).unwrap();
    assert_eq!(s.project(), &keyed);
}
