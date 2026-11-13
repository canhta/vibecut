#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use engine::{Command, EditError, Placement, Session, TrimEdge};
use project::{
    AnimationPath, Animations, AudioParams, Item, ItemId, ItemParams, Keyframe, Param, ParamValue,
    Project, ProjectId, Retime, Track, TrackId, TrackKind, VideoParams,
};
use time::{FrameRate, SourceRange, Speed, TICKS_PER_SECOND, Tick};

const SECOND: i64 = TICKS_PER_SECOND;
const FRAME: i64 = SECOND / 30;

#[test]
fn timeline_group_trim_refuses_unrepresentable_edges_and_source_exhaustion() {
    let mut s = session(
        vec![video(1, 0, range(10 * SECOND, 30 * SECOND))],
        vec![audio(2, 0, range(10 * SECOND, 30 * SECOND))],
        false,
    );
    s.apply(set_speed(&[1, 2], 0.5, true)).unwrap();
    let before = s.project().clone();
    let history = s.history_len();
    for (edge, delta) in [
        (TrimEdge::Start, 1),
        (TrimEdge::End, -1),
        (TrimEdge::End, 2000 * SECOND),
    ] {
        assert!(
            s.apply(Command::TimelineTrim {
                items: vec![ItemId(1), ItemId(2)],
                edge,
                delta: Tick(delta),
            })
            .is_err()
        );
        assert_eq!(s.project(), &before);
        assert_eq!(s.history_len(), history);
    }
}

#[test]
fn reverse_keeps_the_span_undoes_and_refuses_retained_pieces() {
    let mut s = session(
        vec![video(1, 0, range(10 * SECOND, 30 * SECOND))],
        vec![audio(2, 0, range(10 * SECOND, 30 * SECOND))],
        false,
    );
    let span = item(&s, 1).end().unwrap();
    s.apply(set_reverse(&[1, 2], true)).unwrap();
    for id in [1, 2] {
        assert!(item(&s, id).retime.reverse);
        assert_eq!(item(&s, id).end().unwrap(), span, "reverse keeps the span");
    }
    s.apply(set_speed(&[1], 2.0, true)).unwrap();
    assert!(
        item(&s, 1).retime.reverse,
        "a speed change keeps the direction"
    );
    s.undo().unwrap();
    s.undo().unwrap();
    assert!(!item(&s, 1).retime.reverse);
    // A split piece reverses in place: its span and duration stay, and undo restores it.
    let cut = Tick(span.0 / 2);
    s.apply(Command::SplitAt {
        item: ItemId(1),
        at: cut,
    })
    .unwrap();
    let right = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.start == cut)
        .map(|i| i.id.0)
        .unwrap();
    let before = item(&s, right);
    s.apply(set_reverse(&[right], true)).unwrap();
    let flipped = item(&s, right);
    assert!(flipped.retime.reverse);
    assert_eq!(flipped.start, before.start);
    assert_eq!(flipped.duration().unwrap(), before.duration().unwrap());
    s.undo().unwrap();
    assert_eq!(item(&s, right).retime, before.retime);
    assert!(s.apply(set_reverse(&[1, 1], true)).is_err());
    assert!(s.apply(set_reverse(&[], true)).is_err());
    assert!(s.apply(set_reverse(&[999], true)).is_err());
}

fn set_reverse(items: &[u64], reverse: bool) -> Command {
    Command::SetReverse {
        items: items.iter().map(|id| ItemId(*id)).collect(),
        reverse,
    }
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(128))]
    #[test]
    fn timeline_group_keeps_audio_video_aligned_at_rational_rates(
        rate in proptest::sample::select(vec![0.5, 1.0, 2.0, 5.0]),
        movement in 0i64..20, trim in 1i64..20, start_edge in proptest::bool::ANY
    ) {
        let mut s = session(vec![video(1, 0, range(10*SECOND,30*SECOND))],
            vec![audio(2, 0, range(10*SECOND,30*SECOND))], false);
        s.apply(set_speed(&[1,2], rate, true)).unwrap();
        s.apply(Command::TimelineMoveItems {
            items: vec![ItemId(1), ItemId(2)],
            delta: Tick(movement * SECOND),
            placement: None,
        })
        .unwrap();
        proptest::prop_assert_eq!(item(&s,1).start,Tick::ZERO);
        proptest::prop_assert_eq!(item(&s,2).start,Tick::ZERO);
        // Put the visual on a secondary track so horizontal movement is unconstrained.
        s.apply(Command::TimelineMoveItems {
            items: vec![ItemId(1)],
            delta: Tick(0),
            placement: Some(Placement::Above { track: TrackId(0) }),
        })
        .unwrap();
        s.apply(Command::TimelineMoveItems {
            items: vec![ItemId(1), ItemId(2)],
            delta: Tick(movement * SECOND),
            placement: None,
        })
        .unwrap();
        let delta = if start_edge {trim*FRAME} else {-trim*FRAME};
        s.apply(Command::TimelineTrim {
            items: vec![ItemId(1), ItemId(2)],
            edge: if start_edge { TrimEdge::Start } else { TrimEdge::End },
            delta: Tick(delta),
        })
        .unwrap();
        let v = item(&s,1);
        let a = item(&s,2);
        proptest::prop_assert_eq!(v.start,a.start);
        proptest::prop_assert_eq!(v.end().unwrap(),a.end().unwrap());
        proptest::prop_assert_eq!(v.source_range,a.source_range);
        proptest::prop_assert_eq!(v.retime.speed,speed(rate));
        let bytes = project::encode(s.project(),false).unwrap();
        proptest::prop_assert_eq!(&project::decode(&bytes).unwrap().project,s.project().as_ref());
    }
}

fn range(a: i64, b: i64) -> SourceRange {
    SourceRange::new(Tick(a), Tick(b)).unwrap()
}
fn speed(value: f64) -> Speed {
    Speed::from_f64(value).unwrap()
}
fn asset() -> media::SourceAsset {
    let long = Tick(600 * SECOND);
    testkit::source::asset(1, std::env::temp_dir().join("vibecut-retime.mp4"))
        .format("mp4")
        .duration(long)
        .content_hash(media::ContentHash::from_bytes(b"retime"))
        .stream(
            testkit::source::video()
                .time_base(media::TimeBase::new(1, 30000).unwrap())
                .duration(long)
                .start(Tick::ZERO)
                .size(64, 64)
                .build(),
        )
        .stream(
            testkit::source::audio()
                .index(1)
                .time_base(media::TimeBase::new(1, 48000).unwrap())
                .duration(long)
                .start(Tick::ZERO)
                .build(),
        )
        .build()
}
fn video(id: u64, start: i64, source: SourceRange) -> Item {
    Item {
        hidden: false,
        blend_mode: Default::default(),
        retime: Default::default(),
        masks: Vec::new(),
        effects: Vec::new(),
        animations: Default::default(),
        id: ItemId(id),
        source_asset: Some(media::AssetId(1)),
        source_range: source,
        start: Tick(start),
        content: ItemParams::Video(VideoParams {
            fades: Default::default(),
            source_audio_enabled: false,
            volume: 0.0,
            muted: false,
            crop: None,
            cleanup: Default::default(),
            background: project::ClipBackground::None,
        }),
    }
}
fn audio(id: u64, start: i64, source: SourceRange) -> Item {
    Item {
        hidden: false,
        content: ItemParams::Audio(AudioParams {
            fades: Default::default(),
            volume: 0.0,
            muted: false,
            source: Default::default(),
            cleanup: Default::default(),
        }),
        ..video(id, start, source)
    }
}
fn opacity_keys(times: &[i64]) -> Animations {
    let mut param = Param::new(ParamValue::Scalar(1.0)).unwrap();
    for time in times {
        param
            .channel_mut(None)
            .unwrap()
            .put(Keyframe::new(Tick(*time), ParamValue::Scalar(0.5)))
            .unwrap();
    }
    let mut animations = Animations::default();
    animations
        .set(AnimationPath::new("opacity").unwrap(), param)
        .unwrap();
    animations
}
fn session(main: Vec<Item>, audio_items: Vec<Item>, magnet: bool) -> Session {
    let mut data = Project::empty(ProjectId(1), "Retime".into())
        .unwrap()
        .into_data();
    data.canvas.width = 64;
    data.canvas.height = 64;
    data.canvas.fps = FrameRate::FPS_30;
    data.assets.push(asset());
    data.scene_mut().timeline.magnet = magnet;
    data.scene_mut().timeline.main.items = main;
    if !audio_items.is_empty() {
        data.scene_mut().timeline.insert_track(Track {
            id: TrackId(50),
            name: "Audio".into(),
            kind: TrackKind::Audio,
            muted: false,
            hidden: false,
            locked: false,
            items: audio_items,
        });
    }
    Session::new(Project::new(data).unwrap(), 100).unwrap()
}
fn item(s: &Session, id: u64) -> Item {
    s.project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == ItemId(id))
        .unwrap()
        .clone()
}
fn set_speed(items: &[u64], value: f64, maintain_pitch: bool) -> Command {
    Command::SetSpeed {
        items: items.iter().map(|id| ItemId(*id)).collect(),
        speed: speed(value),
        maintain_pitch,
    }
}

#[test]
fn set_speed_adjusts_duration_stretches_keys_and_undoes() {
    for (value, duration, key) in [
        (0.01, 400 * SECOND, 100 * SECOND),
        (0.5, 8 * SECOND, 2 * SECOND),
        (1.0, 4 * SECOND, SECOND),
        (2.0, 2 * SECOND, SECOND / 2),
        (5.0, 4 * SECOND / 5, SECOND / 5),
    ] {
        let mut clip = video(1, 0, range(SECOND, 5 * SECOND));
        clip.animations = opacity_keys(&[SECOND]);
        let mut s = session(vec![clip], vec![], true);
        let before = s.project().clone();
        let outcome = s.apply(set_speed(&[1], value, true)).unwrap();
        assert_eq!(outcome.changed, value != 1.0 || before != *s.project());
        let after = item(&s, 1);
        assert_eq!(
            after.source_range,
            range(SECOND, 5 * SECOND),
            "source window is kept"
        );
        assert_eq!(after.duration().unwrap(), Tick(duration), "{value}x");
        assert_eq!(s.project().duration(), Tick(duration));
        assert_eq!(
            after.retime,
            Retime {
                window: None,
                speed: speed(value),
                maintain_pitch: true,
                reverse: false
            }
        );
        let keys: Vec<_> = after
            .animations
            .get(&AnimationPath::new("opacity").unwrap())
            .unwrap()
            .channel(None)
            .unwrap()
            .keys()
            .iter()
            .map(|k| k.time)
            .collect();
        assert_eq!(keys, vec![Tick(key)], "{value}x keyframe follows the clip");
        if value == 1.0 {
            assert!(
                !outcome.changed,
                "1x with the default pitch policy changes nothing"
            );
            assert_eq!(*s.project(), before);
            continue;
        }
        s.undo().unwrap();
        assert_eq!(
            *s.project(),
            before,
            "undo restores speed, duration and keys"
        );
        s.redo().unwrap();
        assert_eq!(item(&s, 1), after);
    }
}

#[test]
fn trim_and_split_map_through_the_speed_and_move_keys_by_output_time() {
    let mut clip = video(1, 0, range(0, 8 * SECOND));
    clip.retime.speed = speed(2.0);
    clip.animations = opacity_keys(&[SECOND, 3 * SECOND]);
    let mut s = session(vec![clip], vec![], false);
    // 8 s of source at 2x occupy 4 s; a split at output 1.5 s cuts source at 3 s.
    s.apply(Command::SplitAt {
        item: ItemId(1),
        at: Tick(3 * SECOND / 2),
    })
    .unwrap();
    let items = s.project().data().scene().timeline.main.items.clone();
    let [left, right] = items.as_slice() else {
        panic!("two pieces")
    };
    assert_eq!(left.source_range, range(0, 3 * SECOND));
    assert_eq!(right.source_range, range(3 * SECOND, 8 * SECOND));
    assert_eq!(left.end().unwrap(), Tick(3 * SECOND / 2));
    assert_eq!(right.start, left.end().unwrap());
    assert_eq!(right.end().unwrap(), Tick(4 * SECOND));
    assert_eq!(left.retime.speed, right.retime.speed);
    assert_eq!(left.retime.maintain_pitch, right.retime.maintain_pitch);
    assert_eq!(
        left.retime_window().unwrap().source_range,
        right.retime_window().unwrap().source_range
    );
    let times = |item: &Item| -> Vec<Tick> {
        item.animations
            .get(&AnimationPath::new("opacity").unwrap())
            .unwrap()
            .channel(None)
            .unwrap()
            .keys()
            .iter()
            .map(|k| k.time)
            .collect()
    };
    assert_eq!(times(left), vec![Tick(SECOND), Tick(3 * SECOND)]);
    assert_eq!(times(right), vec![Tick(-SECOND / 2), Tick(3 * SECOND / 2)]);
    // Trimming the right piece's start by 0.5 s output drops 1 s of source.
    let right_id = right.id.0;
    s.apply(Command::TrimStart {
        item: ItemId(right_id),
        at: Tick(2 * SECOND),
    })
    .unwrap();
    let trimmed = item(&s, right_id);
    assert_eq!(trimmed.source_range, range(4 * SECOND, 8 * SECOND));
    assert_eq!(trimmed.start, Tick(2 * SECOND));
    assert_eq!(trimmed.end().unwrap(), Tick(4 * SECOND));
    assert_eq!(times(&trimmed), vec![Tick(-SECOND), Tick(SECOND)]);
    // Trimming the end by 0.5 s output drops 1 s of source.
    s.apply(Command::TrimEnd {
        item: ItemId(right_id),
        at: Tick(7 * SECOND / 2),
    })
    .unwrap();
    let trimmed = item(&s, right_id);
    assert_eq!(trimmed.source_range, range(4 * SECOND, 7 * SECOND));
    assert_eq!(trimmed.end().unwrap(), Tick(7 * SECOND / 2));
    // Ripple delete closes exactly the retimed output gap.
    let mut s = session(
        vec![
            {
                let mut a = video(1, 0, range(0, 4 * SECOND));
                a.retime.speed = speed(2.0);
                a
            },
            video(2, 2 * SECOND, range(0, SECOND)),
        ],
        vec![],
        false,
    );
    s.apply(Command::RippleDelete { item: ItemId(1) }).unwrap();
    assert_eq!(item(&s, 2).start, Tick::ZERO);
}

#[test]
fn linked_video_and_audio_stay_in_sync_through_rate_change_trim_and_split() {
    let source = range(2 * SECOND, 12 * SECOND);
    let mut s = session(vec![video(1, 0, source)], vec![audio(2, 0, source)], false);
    s.apply(set_speed(&[1, 2], 1.7, false)).unwrap();
    s.apply(Command::TrimStart {
        item: ItemId(1),
        at: Tick(SECOND),
    })
    .unwrap();
    s.apply(Command::TrimStart {
        item: ItemId(2),
        at: Tick(SECOND),
    })
    .unwrap();
    s.apply(Command::SplitAt {
        item: ItemId(1),
        at: Tick(3 * SECOND),
    })
    .unwrap();
    s.apply(Command::SplitAt {
        item: ItemId(2),
        at: Tick(3 * SECOND),
    })
    .unwrap();
    let pieces = |track: &Track| track.items.clone();
    let timeline = &s.project().data().scene().timeline;
    let (video_pieces, audio_pieces) = (pieces(&timeline.main), pieces(&timeline.audio[0]));
    assert_eq!(video_pieces.len(), 2);
    for (v, a) in video_pieces.iter().zip(&audio_pieces) {
        assert_eq!(
            (v.start, v.source_range, v.retime.speed),
            (a.start, a.source_range, a.retime.speed)
        );
        for step in 0..60 {
            let at = Tick(v.start.0 + step * (v.duration().unwrap().0 / 60));
            let diff = v.source_time_at(at).unwrap().0 - a.source_time_at(at).unwrap().0;
            assert!(diff.abs() <= FRAME, "linked pieces drift {diff} ticks");
        }
    }
}

#[test]
fn speed_edits_refuse_atomically_and_ripple_only_on_a_magnetic_main_track() {
    let overlay = Item {
        hidden: false,
        source_asset: None,
        content: ItemParams::Overlay(project::OverlayParams {
            color: project::Color([1, 2, 3, 255]),
        }),
        ..video(3, 4 * SECOND, range(0, SECOND))
    };
    let mut s = session(
        vec![
            video(1, 0, range(0, 2 * SECOND)),
            video(2, 2 * SECOND, range(0, 2 * SECOND)),
            overlay,
        ],
        vec![],
        false,
    );
    let before = s.project().clone();
    // Slowing item 1 would collide with item 2 on a non-magnetic track.
    assert!(matches!(
        s.apply(set_speed(&[1], 0.5, false)),
        Err(EditError::Overlap(_))
    ));
    assert!(
        s.apply(set_speed(&[3], 2.0, false)).is_err(),
        "generated overlays have no source speed"
    );
    assert!(s.apply(set_speed(&[], 2.0, false)).is_err());
    assert!(s.apply(set_speed(&[1, 1], 2.0, false)).is_err());
    assert!(s.apply(set_speed(&[999], 2.0, false)).is_err());
    assert_eq!(*s.project(), before, "failed edits change nothing");
    // 2 s of source at 5x is 0.4 s; faster leaves a gap on a free track.
    s.apply(set_speed(&[1], 5.0, false)).unwrap();
    assert_eq!(item(&s, 2).start, Tick(2 * SECOND));
    // A magnetic main track closes and opens gaps by the duration change.
    let mut s = session(
        vec![
            video(1, 0, range(0, 2 * SECOND)),
            video(2, 2 * SECOND, range(0, 2 * SECOND)),
        ],
        vec![],
        true,
    );
    s.apply(set_speed(&[1], 0.5, false)).unwrap();
    assert_eq!(
        (item(&s, 1).end().unwrap(), item(&s, 2).start),
        (Tick(4 * SECOND), Tick(4 * SECOND))
    );
    s.undo().unwrap();
    assert_eq!(item(&s, 2).start, Tick(2 * SECOND));
    // Out-of-range speeds cannot be expressed on the wire.
    for bad in [0.0, 0.009, 5.001, -1.0] {
        assert!(
            serde_json::from_value::<Command>(serde_json::json!({
                "command": "timeline.set_speed",
                "params": {"items": [1], "speed": bad, "maintain_pitch": false}
            }))
            .is_err(),
            "{bad}"
        );
    }
    // Four source ticks at 5x would play for no output tick.
    let mut s = session(vec![video(1, 0, range(0, 4))], vec![], false);
    assert!(s.apply(set_speed(&[1], 5.0, false)).is_err());
}

#[test]
fn split_inside_retimed_transition_preserves_output_span() {
    let mut a = video(1, 0, range(0, 4 * SECOND));
    let mut b = video(2, 2 * SECOND, range(4 * SECOND, 8 * SECOND));
    a.retime.speed = speed(2.0);
    b.retime.speed = speed(2.0);
    let mut s = session(vec![a, b], vec![], true);
    let track = s.project().data().scene().timeline.main.id;
    s.apply(Command::AddTransition {
        track,
        from: ItemId(1),
        to: ItemId(2),
        kind: "cross_dissolve".into(),
        duration: Tick(2 * SECOND),
        alignment: project::TransitionAlignment::Center,
    })
    .unwrap();
    let before = s
        .project()
        .data()
        .scene()
        .timeline
        .transitions
        .first()
        .unwrap()
        .clone();
    s.apply(Command::SplitAt {
        item: ItemId(1),
        at: Tick(3 * SECOND / 2),
    })
    .unwrap();
    let after = s
        .project()
        .data()
        .scene()
        .timeline
        .transitions
        .first()
        .unwrap();
    assert_eq!(
        (after.start, after.duration),
        (before.start, before.duration),
        "split must retain original transition progress at every output tick"
    );
}

struct SplitHooks;
struct SplitLease;
impl render::Reservation for SplitLease {}
impl render::ExecutionHooks for SplitHooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn reserve(
        &self,
        _: render::MemoryKind,
        _: u64,
    ) -> render::Result<Box<dyn render::Reservation>> {
        Ok(Box::new(SplitLease))
    }
}
struct SplitSource;
impl render::SourceProvider for SplitSource {
    fn status(
        &mut self,
        _: &media::SourceAsset,
        _: bool,
        _: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::UnavailableReason>> {
        Ok(None)
    }
    fn video(
        &mut self,
        _: &media::SourceAsset,
        _: ItemId,
        at: Tick,
        h: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::Image>> {
        // Every source tick changes the picture; item identity deliberately does not.
        let x = (at.0.rem_euclid(1_000_003) as f32) / 1_000_003.;
        Ok(Some(render::Image::new(2, 2, [x, x * 0.5, 1. - x, 1.], h)?))
    }
    fn audio(
        &mut self,
        _: &media::SourceAsset,
        _: ItemId,
        at: i64,
        n: usize,
        rate: u32,
        h: &dyn render::ExecutionHooks,
    ) -> render::Result<render::AudioBlock> {
        let mut b = render::AudioBlock::new(rate, 1, n, render::MemoryKind::Decoded, h)?;
        for plane in b.planes_mut() {
            for (i, v) in plane.iter_mut().enumerate() {
                *v = (((at + i as i64) as f64 * 0.057).sin() * 0.1) as f32;
            }
        }
        Ok(b)
    }
}

fn split_pictures(p: &Project, times: &[Tick]) -> Vec<Vec<[f32; 4]>> {
    let mut settings = render::RenderSettings::from_project(p);
    settings.width = 2;
    settings.height = 2;
    let mut r = render::Renderer::new(p, settings).unwrap();
    times
        .iter()
        .map(|t| {
            r.render_frame(*t, &mut SplitSource, &SplitHooks)
                .unwrap()
                .pixels()
                .to_vec()
        })
        .collect()
}

#[test]
fn transition_splits_preserve_every_frame_mapping_history_and_persistence() {
    use project::TransitionAlignment::{Center, End, Start};
    for rate in [0.5, 1., 2., 0.333, 1.501, 5.] {
        for alignment in [Start, Center, End] {
            for definition in project::TRANSITION_DEFINITIONS {
                let mut a = video(1, 0, range(4 * SECOND, 8 * SECOND));
                a.retime.speed = speed(rate);
                let cut = a.end().unwrap().0;
                let mut opacity = Param::new(ParamValue::Scalar(0.2)).unwrap();
                for (t, value) in [(0, 0.2), (cut / 3, 0.9), (cut, 0.4)] {
                    opacity
                        .channel_mut(None)
                        .unwrap()
                        .put(Keyframe::new(Tick(t), ParamValue::Scalar(value)))
                        .unwrap();
                }
                a.animations
                    .set(AnimationPath::new("opacity").unwrap(), opacity)
                    .unwrap();
                let mut b = video(2, cut, range(12 * SECOND, 16 * SECOND));
                b.retime.speed = speed(rate);
                let mut s = session(vec![a, b], vec![], true);
                s.apply(Command::AddTransition {
                    track: s.project().data().scene().timeline.main.id,
                    from: ItemId(1),
                    to: ItemId(2),
                    kind: definition.kind.into(),
                    duration: Tick(cut / 2),
                    alignment,
                })
                .unwrap();
                let original = s.project().clone();
                for id in [ItemId(1), ItemId(2)] {
                    assert!(
                        s.apply(Command::SplitAt {
                            item: id,
                            at: Tick(cut)
                        })
                        .is_err()
                    );
                    assert_eq!(s.project(), &original);
                }
                let tr = &original.data().scene().timeline.transitions[0];
                let end = original.duration().0;
                let mut times: Vec<_> = (0..end).step_by(FRAME as usize).map(Tick).collect();
                // Include both sides of span boundaries and odd, non-frame cut ticks.
                let cuts = [
                    tr.start.0,
                    tr.start.0 + tr.duration.0 / 4 + 1,
                    tr.start.0 + tr.duration.0 * 3 / 4 + 1,
                    tr.end().unwrap().0,
                    cut / 3 + 1,
                    cut + cut / 3 + 1,
                ];
                for t in cuts {
                    times.extend([Tick(t - 1), Tick(t), Tick(t + 1)]);
                }
                times.retain(|t| t.0 >= 0 && t.0 < end);
                let expected = split_pictures(&original, &times);
                for at in cuts {
                    if at <= 0 || at >= end || at == cut {
                        continue;
                    }
                    let id = if at < cut { ItemId(1) } else { ItemId(2) };
                    s.apply(Command::SplitAt {
                        item: id,
                        at: Tick(at),
                    })
                    .unwrap();
                    // Split the remaining larger piece again, without undoing the first cut.
                    let piece = s
                        .project()
                        .data()
                        .scene()
                        .timeline
                        .main
                        .items
                        .iter()
                        .filter(|i| {
                            if at < cut {
                                i.start.0 < cut
                            } else {
                                i.start.0 >= cut
                            }
                        })
                        .max_by_key(|i| i.duration().unwrap())
                        .unwrap();
                    let second = piece.start.0 + piece.duration().unwrap().0 / 2 + 1;
                    let id = piece.id;
                    let once = s.project().clone();
                    s.apply(Command::SplitAt {
                        item: id,
                        at: Tick(second),
                    })
                    .unwrap();
                    let twice = s.project().clone();
                    let after = &twice.data().scene().timeline.transitions[0];
                    assert_eq!(
                        (after.start, after.duration, after.alignment),
                        (tr.start, tr.duration, tr.alignment),
                        "{rate} {alignment:?} {at}"
                    );
                    assert_eq!(
                        split_pictures(&twice, &times),
                        expected,
                        "{rate} {alignment:?} {} {at}",
                        definition.kind
                    );
                    let boundary = [Tick(second - 1), Tick(second), Tick(second + 1)];
                    assert_eq!(
                        split_pictures(&twice, &boundary),
                        split_pictures(&original, &boundary)
                    );
                    let loaded = project::decode(&project::encode(&twice, false).unwrap())
                        .unwrap()
                        .project;
                    assert_eq!(split_pictures(&loaded, &times), expected);
                    s.undo().unwrap();
                    assert_eq!(s.project(), &once);
                    s.redo().unwrap();
                    assert_eq!(s.project(), &twice);
                    s.undo().unwrap();
                    s.undo().unwrap();
                    assert_eq!(s.project(), &original);
                }
                for (id, at) in [(ItemId(1), cut * 3 / 4 + 1), (ItemId(2), cut + cut / 4 + 1)] {
                    s.apply(Command::SplitAt {
                        item: id,
                        at: Tick(at),
                    })
                    .unwrap();
                }
                assert_eq!(
                    split_pictures(s.project(), &times),
                    expected,
                    "both sides split"
                );
                let loaded = project::decode(&project::encode(s.project(), false).unwrap())
                    .unwrap()
                    .project;
                assert_eq!(split_pictures(&loaded, &times), expected);
                s.undo().unwrap();
                s.undo().unwrap();
                assert_eq!(s.project(), &original);
            }
        }
    }
}

#[test]
fn split_continuations_refuse_corrupt_mapping_and_piece_references() {
    use serde_json::json;
    let mut a = video(1, 0, range(4 * SECOND, 8 * SECOND));
    let mut b = video(2, 2 * SECOND, range(12 * SECOND, 16 * SECOND));
    a.retime.speed = speed(2.);
    b.retime.speed = speed(2.);
    let mut s = session(vec![a, b], vec![], true);
    s.apply(Command::AddTransition {
        track: s.project().data().scene().timeline.main.id,
        from: ItemId(1),
        to: ItemId(2),
        kind: "cross_dissolve".into(),
        duration: Tick(2 * SECOND),
        alignment: project::TransitionAlignment::Center,
    })
    .unwrap();
    s.apply(Command::SplitAt {
        item: ItemId(1),
        at: Tick(3 * SECOND / 2),
    })
    .unwrap();
    let wire: serde_json::Value =
        serde_json::from_slice(&project::encode(s.project(), false).unwrap()).unwrap();
    for (field, value) in [
        ("offset", json!(-1)),
        ("offset", json!(1)),
        ("offset", json!(i64::MAX)),
        ("duration", json!(0)),
        ("duration", json!(-1)),
        ("duration", json!(i64::MAX)),
        ("source_range", json!({"start":-1,"end":8 * SECOND})),
        (
            "source_range",
            json!({"start":4 * SECOND,"end":601 * SECOND}),
        ),
    ] {
        let mut bad = wire.clone();
        bad["project"]["scenes"][0]["timeline"]["main"]["items"][0]["retime"]["window"][field] =
            value;
        assert!(
            project::decode(&serde_json::to_vec(&bad).unwrap()).is_err(),
            "{field}"
        );
    }
    for ids in [json!([999]), json!([1, 1]), json!([2])] {
        let mut bad = wire.clone();
        bad["project"]["scenes"][0]["timeline"]["transitions"][0]["from_pieces"] = ids;
        assert!(project::decode(&serde_json::to_vec(&bad).unwrap()).is_err());
    }
    let before = s.project().clone();
    let id = before.data().scene().timeline.transitions[0].from;
    let mapping = item(&s, id.0);
    s.apply(set_speed(&[id.0], 2., false)).unwrap();
    assert_eq!(
        item(&s, id.0).retime.window,
        mapping.retime.window,
        "pitch changes retain the mapping"
    );
    s.undo().unwrap();
    assert_eq!(s.project(), &before);
}

fn split_audio(p: &Project) -> Vec<Vec<f32>> {
    let count = p
        .duration()
        .to_units(48000, time::Rounding::Nearest)
        .unwrap() as usize;
    let mut m = render::AudioMixer::new(p, 1, 5, &SplitHooks).unwrap();
    m.mix(0, count, &mut SplitSource, &SplitHooks)
        .unwrap()
        .planes()
        .to_vec()
}

#[test]
fn retimed_fades_and_audio_replay_survive_repeated_cuts_history_and_persistence() {
    for rate in [0.5, 1., 2., 0.333, 1.501, 5.] {
        for maintain_pitch in [false, true] {
            for curve in [
                project::FadeCurve::Linear,
                project::FadeCurve::EqualPower,
                project::FadeCurve::Exponential,
            ] {
                let mut a = audio(1, 0, range(SECOND, SECOND + SECOND / 5));
                let start = SECOND / 7 + 1;
                a.start = Tick(start);
                a.retime.speed = speed(rate);
                a.retime.maintain_pitch = maintain_pitch;
                let duration = a.duration().unwrap();
                if let ItemParams::Audio(p) = &mut a.content {
                    p.fades
                        .set(project::FadeEdge::In, Tick(duration.0 / 3), curve, duration)
                        .unwrap();
                    p.fades
                        .set(
                            project::FadeEdge::Out,
                            Tick(duration.0 / 3),
                            curve,
                            duration,
                        )
                        .unwrap();
                }
                let mut s = session(vec![], vec![a], false);
                let original = s.project().clone();
                let expected = split_audio(&original);
                for at in [
                    duration.0 / 6 + 1,
                    duration.0 / 3,
                    duration.0 * 2 / 3,
                    duration.0 * 5 / 6 + 1,
                ] {
                    s.apply(Command::SplitAt {
                        item: ItemId(1),
                        at: Tick(start + at),
                    })
                    .unwrap();
                    let once = s.project().clone();
                    let piece = s
                        .project()
                        .data()
                        .scene()
                        .timeline
                        .iter()
                        .flat_map(|t| &t.items)
                        .max_by_key(|i| i.duration().unwrap())
                        .unwrap();
                    s.apply(Command::SplitAt {
                        item: piece.id,
                        at: Tick(piece.start.0 + piece.duration().unwrap().0 / 2 + 1),
                    })
                    .unwrap();
                    let twice = s.project().clone();
                    assert_eq!(
                        split_audio(&twice),
                        expected,
                        "{rate} {maintain_pitch} {curve:?} {at}"
                    );
                    let loaded = project::decode(&project::encode(&twice, false).unwrap())
                        .unwrap()
                        .project;
                    assert_eq!(split_audio(&loaded), expected);
                    s.undo().unwrap();
                    assert_eq!(s.project(), &once);
                    s.redo().unwrap();
                    assert_eq!(s.project(), &twice);
                    s.undo().unwrap();
                    s.undo().unwrap();
                    assert_eq!(s.project(), &original);
                }
            }
        }
    }
}
