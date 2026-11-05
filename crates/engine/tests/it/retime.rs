#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use engine::{Command, EditError, Session};
use project::{
    AnimationPath, Animations, AudioParams, Item, ItemId, ItemParams, Keyframe, Param, ParamValue,
    Project, ProjectId, Retime, Track, TrackId, TrackKind, VideoParams,
};
use time::{FrameRate, SourceRange, Speed, TICKS_PER_SECOND, Tick};

const SECOND: i64 = TICKS_PER_SECOND;
const FRAME: i64 = SECOND / 30;

fn range(a: i64, b: i64) -> SourceRange {
    SourceRange::new(Tick(a), Tick(b)).unwrap()
}
fn speed(value: f64) -> Speed {
    Speed::from_f64(value).unwrap()
}
fn asset() -> media::SourceAsset {
    let long = Tick(600 * SECOND);
    media::SourceAsset::try_from(media::AssetData {
        ephemeral: false,
        id: media::AssetId(1),
        path: std::env::temp_dir().join("vibecut-retime.mp4"),
        content_hash: media::ContentHash::from_bytes(b"retime"),
        probe: media::SourceProbe {
            format: "mp4".into(),
            duration: Some(long),
            streams: vec![
                media::SourceStream {
                    index: 0,
                    kind: media::StreamKind::Video,
                    codec: "h264".into(),
                    time_base: media::TimeBase::new(1, 30000).unwrap(),
                    duration: Some(long),
                    start: Some(Tick::ZERO),
                    frame_rate: Some(FrameRate::FPS_30),
                    frame_count: None,
                    variable_frame_rate: None,
                    video: Some(media::VideoInfo {
                        width: 64,
                        height: 64,
                        color: media::ColorInfo::default(),
                        bit_depth: Some(8),
                    }),
                    audio: None,
                },
                media::SourceStream {
                    index: 1,
                    kind: media::StreamKind::Audio,
                    codec: "aac".into(),
                    time_base: media::TimeBase::new(1, 48000).unwrap(),
                    duration: Some(long),
                    start: Some(Tick::ZERO),
                    frame_rate: None,
                    frame_count: None,
                    variable_frame_rate: None,
                    video: None,
                    audio: Some(media::AudioInfo {
                        sample_rate: 48000,
                        channels: 2,
                        initial_padding: 0,
                        trailing_padding: 0,
                    }),
                },
            ],
        },
    })
    .unwrap()
}
fn video(id: u64, start: i64, source: SourceRange) -> Item {
    Item {
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
        }),
    }
}
fn audio(id: u64, start: i64, source: SourceRange) -> Item {
    Item {
        content: ItemParams::Audio(AudioParams {
            fades: Default::default(),
            volume: 0.0,
            muted: false,
            source: Default::default(),
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
                speed: speed(value),
                maintain_pitch: true
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
    assert_eq!(left.retime, right.retime);
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
