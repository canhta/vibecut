#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use crate::project::{empty, range, source};
use project::*;
use proptest::prelude::*;
use time::{Speed, TICKS_PER_SECOND, Tick};

const SPEEDS: [u32; 5] = [100, 500, 1000, 2000, 5000];
/// The bound speeds, 0.1x and 100x, with the whole-number rates between.
const BOUND_SPEEDS: [u32; 6] = [100, 500, 1000, 2000, 5000, 100_000];

#[test]
fn rate_duration_and_source_range_agree_at_every_boundary_speed() {
    let second = TICKS_PER_SECOND;
    for thousandths in BOUND_SPEEDS {
        let item = testkit::item::item(
            ItemId(1),
            ItemParams::Video(VideoParams {
                source_audio_enabled: true,
                ..VideoParams::plain()
            }),
        )
        .asset(media::AssetId(3))
        .source(range(7 * second, 17 * second))
        .start(Tick(3 * second))
        .retime(Retime {
            window: None,
            speed: Speed::from_thousandths(thousandths).unwrap(),
            maintain_pitch: false,
            reverse: false,
            curve: None,
        })
        .build();
        let speed = item.retime.speed;
        let duration = item.duration().unwrap();
        assert_eq!(
            duration,
            Tick(10 * second * 1000 / i64::from(thousandths)),
            "{speed}"
        );
        assert_eq!(item.end().unwrap(), Tick(3 * second + duration.0));
        // The last output tick still shows the final source tick, never past it.
        let last = item
            .source_time_at(Tick(item.end().unwrap().0 - 1))
            .unwrap();
        assert!(
            last < Tick(17 * second) && last >= Tick(17 * second - 5 * 1000),
            "{speed}"
        );
        assert_eq!(item.source_time_at(item.start).unwrap(), Tick(7 * second));
        // Source -> output -> source stays within a few ticks.
        for offset in [0, 1, second / 3, 9 * second] {
            let at = item.output_time_of(Tick(7 * second + offset)).unwrap();
            let back = item.source_time_at(at).unwrap();
            assert!(
                (back.0 - 7 * second - offset).abs()
                    <= 5.max(i64::from(thousandths.div_ceil(1000))),
                "{speed} {offset}"
            );
        }
    }
}

fn check_split(item: &Item, at: Tick) {
    let (left, right) = item.split_at(at).unwrap();
    let left_item = Item {
        hidden: false,
        source_range: left.source_range,
        start: left.start,
        retime: left.retime,
        ..item.clone()
    };
    let right_item = Item {
        hidden: false,
        source_range: right.source_range,
        start: right.start,
        retime: right.retime,
        ..item.clone()
    };
    assert_eq!(left.start, item.start);
    assert_eq!(left_item.end().unwrap(), at);
    assert_eq!(right.start, at, "pieces touch at the requested tick");
    assert_eq!(
        right_item.end().unwrap(),
        item.end().unwrap(),
        "no lost output ticks"
    );
    assert_eq!(left.source_range.start(), item.source_range.start());
    assert_eq!(left.source_range.end(), right.source_range.start());
    assert_eq!(right.source_range.end(), item.source_range.end());
    for t in [
        item.start,
        Tick(at.0 - 1),
        at,
        Tick(at.0 + 1),
        Tick(item.end().unwrap().0 - 1),
    ] {
        if t >= item.end().unwrap() {
            continue;
        }
        let piece = if t < at { &left_item } else { &right_item };
        assert_eq!(
            piece.source_time_at(t).unwrap(),
            item.source_time_at(t).unwrap()
        );
    }
    for piece in [&left_item, &right_item] {
        let middle = Tick(piece.start.0 + piece.duration().unwrap().0 / 2);
        for cut in [piece.trim_start(middle), piece.trim_end(middle)]
            .into_iter()
            .flatten()
        {
            let trimmed = Item {
                hidden: false,
                start: cut.start,
                source_range: cut.source_range,
                retime: cut.retime,
                ..piece.clone()
            };
            assert!(trimmed.start == piece.start || trimmed.end().unwrap() == piece.end().unwrap());
            for t in [trimmed.start, Tick(trimmed.end().unwrap().0 - 1)] {
                assert_eq!(
                    trimmed.source_time_at(t).unwrap(),
                    item.source_time_at(t).unwrap(),
                    "trimmed split keeps phase"
                );
            }
        }
    }
}

#[test]
fn split_and_trim_respect_retime_at_boundary_and_odd_cuts() {
    let second = TICKS_PER_SECOND;
    let frame = second / 30;
    for thousandths in SPEEDS {
        let item = testkit::item::item(
            ItemId(1),
            ItemParams::Video(VideoParams {
                source_audio_enabled: true,
                ..VideoParams::plain()
            }),
        )
        .asset(media::AssetId(3))
        .source(range(0, 4 * second))
        .start(Tick(10 * second))
        .retime(Retime {
            window: None,
            speed: Speed::from_thousandths(thousandths).unwrap(),
            maintain_pitch: false,
            reverse: false,
            curve: None,
        })
        .build();
        let end = item.end().unwrap();
        let length = end.0 - item.start.0;
        for local in [
            1,
            2,
            7,
            99,
            frame,
            length / 3,
            length / 2 + 1,
            length - frame,
            length - 1,
        ] {
            if local <= 0 || local >= length {
                continue;
            }
            let at = Tick(item.start.0 + local);
            if item.split_at(at).is_ok() {
                check_split(&item, at);
            } else {
                // Only a piece shorter than one source tick is refused.
                assert!(local < 101 || length - local < 6, "{thousandths} {local}");
            }
            if let Ok(cut) = item.trim_start(at) {
                assert!(
                    cut.start >= at && cut.start.0 - at.0 <= 101,
                    "{thousandths} {local}"
                );
                assert_eq!(cut.source_range.end(), item.source_range.end());
                let trimmed = Item {
                    hidden: false,
                    start: cut.start,
                    source_range: cut.source_range,
                    ..item.clone()
                };
                assert_eq!(trimmed.end().unwrap(), end, "the end edge never moves");
            }
            if let Ok(cut) = item.trim_end(at) {
                let trimmed = Item {
                    hidden: false,
                    source_range: cut.source_range,
                    ..item.clone()
                };
                let trimmed_end = trimmed.end().unwrap();
                assert!(trimmed_end <= at && at.0 - trimmed_end.0 <= 101);
                assert_eq!(cut.start, item.start);
            }
        }
        assert!(item.split_at(item.start).is_err());
        assert!(item.split_at(end).is_err());
        assert!(item.trim_start(end).is_err());
        assert!(item.trim_end(item.start).is_err());
    }
    // Unity speed keeps the exact tick arithmetic.
    let unity = testkit::item::item(
        ItemId(1),
        ItemParams::Video(VideoParams {
            source_audio_enabled: true,
            ..VideoParams::plain()
        }),
    )
    .asset(media::AssetId(3))
    .source(range(100, 200))
    .start(Tick(5))
    .retime(Retime {
        window: None,
        speed: Speed::from_thousandths(1000).unwrap(),
        maintain_pitch: false,
        reverse: false,
        curve: None,
    })
    .build();
    let (left, right) = unity.split_at(Tick(45)).unwrap();
    assert_eq!(
        (left.source_range, right.source_range, right.start),
        (range(100, 140), range(140, 200), Tick(45))
    );
    assert_eq!(
        unity.trim_start(Tick(15)).unwrap().source_range,
        range(110, 200)
    );
    assert_eq!(
        unity.trim_end(Tick(65)).unwrap().source_range,
        range(100, 160)
    );
}

proptest! {
    #[test]
    fn split_pieces_partition_the_source_without_overlap(
        thousandths in 100u32..=100_000,
        source_ticks in 20i64..4_000_000_000_000,
        cut in 1i64..4_000_000_000_000,
        start in 0i64..1_000_000_000,
    ) {
        let item = testkit::item::item(ItemId(1), ItemParams::Video(VideoParams { source_audio_enabled: true, ..VideoParams::plain() })).asset(media::AssetId(3)).source(range(0, source_ticks)).start(Tick(start)).retime(Retime { window: None, speed: Speed::from_thousandths(thousandths).unwrap(), maintain_pitch: false, reverse: false, curve: None }).build();
        let length = item.duration().unwrap().0;
        prop_assume!(length >= 2);
        let at = Tick(start + 1 + cut % (length - 1));
        if item.split_at(at).is_ok() {
            check_split(&item, at);
        }
    }
}

#[test]
fn persistence_roundtrips_speed_and_pitch_and_refuses_invalid_retime() {
    let dir = tempfile::tempdir().unwrap();
    let mut data = empty().into_data();
    data.assets.push(source(dir.path().join("clip.mp4")));
    let mut video = testkit::item::item(
        ItemId(1),
        ItemParams::Video(VideoParams {
            source_audio_enabled: true,
            ..VideoParams::plain()
        }),
    )
    .asset(media::AssetId(3))
    .source(range(0, 400))
    .start(Tick(0))
    .retime(Retime {
        window: None,
        speed: Speed::from_thousandths(500).unwrap(),
        maintain_pitch: false,
        reverse: false,
        curve: None,
    })
    .build();
    video.retime.maintain_pitch = true;
    data.scene_mut().timeline.main.items.push(video);
    let mut audio = testkit::item::item(
        ItemId(1),
        ItemParams::Video(VideoParams {
            source_audio_enabled: true,
            ..VideoParams::plain()
        }),
    )
    .asset(media::AssetId(3))
    .source(range(0, 300))
    .start(Tick(0))
    .retime(Retime {
        window: None,
        speed: Speed::from_thousandths(2500).unwrap(),
        maintain_pitch: false,
        reverse: false,
        curve: None,
    })
    .build();
    audio.id = ItemId(2);
    audio.content = ItemParams::Audio(AudioParams::default());
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(9),
        name: "Audio".into(),
        kind: TrackKind::Audio,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![audio],
    });
    let project = Project::new(data).unwrap();
    assert_eq!(project.duration(), Tick(800));
    let bytes = encode(&project, false).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(
        text.contains(r#""retime":{"speed":0.5,"maintain_pitch":true,"reverse":false}"#),
        "{text}"
    );
    assert_eq!(decode(&bytes).unwrap().project, project);

    let mut text_item = testkit::item::item(
        ItemId(1),
        ItemParams::Video(VideoParams {
            source_audio_enabled: true,
            ..VideoParams::plain()
        }),
    )
    .asset(media::AssetId(3))
    .source(range(0, 10))
    .start(Tick(0))
    .retime(Retime {
        window: None,
        speed: Speed::from_thousandths(1000).unwrap(),
        maintain_pitch: false,
        reverse: false,
        curve: None,
    })
    .build();
    text_item.content = ItemParams::Text(TextParams::new("x"));
    text_item.source_asset = None;
    text_item.retime = Retime::default();
    let mut data = empty().into_data();
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(4),
        name: "Text".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![text_item],
    });
    assert!(Project::new(data.clone()).is_ok());
    for (speed, pitch) in [(2000, true), (1000, false)] {
        let mut candidate = data.clone();
        let item = candidate
            .scene_mut()
            .timeline
            .iter_mut()
            .flat_map(|t| &mut t.items)
            .next()
            .unwrap();
        item.retime = Retime {
            window: None,
            speed: Speed::from_thousandths(speed).unwrap(),
            maintain_pitch: pitch,
            reverse: false,
            curve: None,
        };
        assert!(
            Project::new(candidate).is_err(),
            "non-media item must stay at 1x"
        );
    }
    // 4 source ticks at 5x play for less than one output tick.
    let mut data = empty().into_data();
    data.assets.push(source(dir.path().join("clip.mp4")));
    data.scene_mut().timeline.main.items.push(
        testkit::item::item(
            ItemId(1),
            ItemParams::Video(VideoParams {
                source_audio_enabled: true,
                ..VideoParams::plain()
            }),
        )
        .asset(media::AssetId(3))
        .source(range(0, 4))
        .start(Tick(0))
        .retime(Retime {
            window: None,
            speed: Speed::from_thousandths(5000).unwrap(),
            maintain_pitch: false,
            reverse: false,
            curve: None,
        })
        .build(),
    );
    assert!(Project::new(data).is_err());
}

#[test]
fn long_retime_mapping_keeps_audio_and_video_within_one_sample_grid() {
    use time::Rounding;
    for thousandths in [100, 333, 500, 1000, 2000, 5000, 100_000] {
        let item = testkit::item::item(
            ItemId(1),
            ItemParams::Video(VideoParams {
                source_audio_enabled: true,
                ..VideoParams::plain()
            }),
        )
        .asset(media::AssetId(3))
        .source(range(7 * TICKS_PER_SECOND + 123, 7200 * TICKS_PER_SECOND))
        .start(Tick(3 * TICKS_PER_SECOND + 456))
        .retime(Retime {
            window: None,
            speed: Speed::from_thousandths(thousandths).unwrap(),
            maintain_pitch: false,
            reverse: false,
            curve: None,
        })
        .build();
        let rate = 48000;
        let origin = item
            .source_range
            .start()
            .to_units(rate, Rounding::Nearest)
            .unwrap();
        let start = item.start.to_units(rate, Rounding::Nearest).unwrap();
        let end = item
            .end()
            .unwrap()
            .to_units(rate, Rounding::Nearest)
            .unwrap();
        for n in [start, start + (end - start) / 2, end - 1] {
            let at = Tick::from_units(n, rate, Rounding::Nearest).unwrap();
            let video = item.source_time_at(at).unwrap();
            let local_audio = Tick::from_units(n - start, rate, Rounding::Nearest).unwrap();
            let audio = Tick::from_units(origin, rate, Rounding::Nearest).unwrap()
                + item
                    .retime
                    .speed
                    .source_offset(local_audio, Rounding::Floor)
                    .unwrap();
            let max_error = TICKS_PER_SECOND / rate * 6;
            assert!(
                (audio.0 - video.0).abs() <= max_error,
                "speed {thousandths}, n {n}"
            );
        }
    }
}

#[test]
fn reversed_clip_plays_its_source_backwards_and_cuts_keep_the_mapping() {
    let second = TICKS_PER_SECOND;
    for thousandths in SPEEDS {
        let mut item = testkit::item::item(
            ItemId(1),
            ItemParams::Video(VideoParams {
                source_audio_enabled: true,
                ..VideoParams::plain()
            }),
        )
        .asset(media::AssetId(3))
        .source(range(0, 4 * second))
        .start(Tick(10 * second))
        .retime(Retime {
            window: None,
            speed: Speed::from_thousandths(thousandths).unwrap(),
            maintain_pitch: false,
            reverse: false,
            curve: None,
        })
        .build();
        item.retime.reverse = true;
        let end = item.end().unwrap();
        let length = end.0 - item.start.0;
        // The first output tick shows the last source tick and the mapping only falls.
        assert_eq!(
            item.source_time_at(item.start).unwrap(),
            Tick(4 * second - 1),
            "{thousandths}"
        );
        let mut previous = Tick(4 * second);
        for step in 0..200 {
            let at = Tick(item.start.0 + length * step / 200);
            let source = item.source_time_at(at).unwrap();
            assert!(
                source <= previous && source >= Tick(0),
                "{thousandths} {step}"
            );
            previous = source;
        }
        // The last output tick stays within a few ticks of the source start.
        assert!(
            item.source_time_at(Tick(end.0 - 1)).unwrap().0 <= 10,
            "{thousandths}"
        );
        // Source -> output -> source is exact: the first output tick at or before a source tick.
        for local in [0, 1, second / 3] {
            let source = item.source_time_at(Tick(item.start.0 + local)).unwrap();
            let at = item.output_time_of(source).unwrap();
            assert_eq!(
                item.source_time_at(at).unwrap(),
                source,
                "{thousandths} {local}"
            );
        }
        for local in [1, 7, 99, length / 3] {
            let at = Tick(item.start.0 + local);
            let (left, right) = item.split_at(at).unwrap();
            assert!(left.retime.reverse && right.retime.reverse);
            assert_eq!(item.trim_start(at).unwrap(), right, "{thousandths} {local}");
            assert_eq!(item.trim_end(at).unwrap(), left, "{thousandths} {local}");
            let left_item = Item {
                source_range: left.source_range,
                start: left.start,
                retime: left.retime,
                ..item.clone()
            };
            let right_item = Item {
                source_range: right.source_range,
                start: right.start,
                retime: right.retime,
                ..item.clone()
            };
            assert_eq!(left_item.end().unwrap(), at);
            assert_eq!(right_item.end().unwrap(), end, "no lost output ticks");
            assert_eq!(left.source_range.end(), item.source_range.end());
            assert_eq!(right.source_range.start(), item.source_range.start());
            // Earlier output shows later source; the pieces meet within one tick.
            assert!(right.source_range.end().0 <= left.source_range.start().0 + 1);
            for t in [item.start, Tick(at.0 - 1), at, Tick(end.0 - 1)] {
                let piece = if t < at { &left_item } else { &right_item };
                assert_eq!(
                    piece.source_time_at(t).unwrap(),
                    item.source_time_at(t).unwrap(),
                    "{thousandths} {local}"
                );
            }
        }
        // Frame index rows for 30 fps numbered frames 0..120 at whole-speed rates:
        // output frame n shows source frame 119 - k*n, so no frame is duplicated or dropped.
        let frame = second / 30;
        if let Some(k) = [1i64, 2]
            .into_iter()
            .find(|k| i64::from(thousandths) == k * 1000)
        {
            for n in 0..120 / k {
                let source = item.source_time_at(Tick(item.start.0 + n * frame)).unwrap();
                assert_eq!(source.0 / frame, 119 - k * n, "{thousandths} frame {n}");
            }
        }
        // A forward split piece flips in place: its window is mirrored, its span
        // and duration stay, and flipping back restores it exactly.
        let mut forward = item.clone();
        forward.retime.reverse = false;
        let (_, right) = forward.split_at(Tick(item.start.0 + length / 3)).unwrap();
        let piece = Item {
            source_range: right.source_range,
            start: right.start,
            retime: right.retime,
            ..forward.clone()
        };
        let mut flipped = piece.clone();
        flipped.set_reversed(true).unwrap();
        assert!(flipped.retime.reverse);
        assert_eq!(flipped.start, piece.start);
        assert_eq!(flipped.duration().unwrap(), piece.duration().unwrap());
        assert_eq!(flipped.end().unwrap(), piece.end().unwrap());
        // The same source span, within the floored rounding of the mapping.
        let slack = 2 * i64::from(thousandths.div_ceil(1000)) + 2;
        assert!((flipped.source_range.start().0 - piece.source_range.start().0).abs() <= slack);
        assert!((flipped.source_range.end().0 - piece.source_range.end().0).abs() <= slack);
        flipped.set_reversed(false).unwrap();
        assert_eq!(flipped.retime, piece.retime, "{thousandths}");
        assert_eq!(flipped.source_range, piece.source_range, "{thousandths}");

        let json = serde_json::to_string(&item.retime).unwrap();
        assert_eq!(serde_json::from_str::<Retime>(&json).unwrap(), item.retime);
    }
}

fn segments(table: &[(u16, u32)]) -> Vec<SpeedSegment> {
    table
        .iter()
        .map(|&(span, thousandths)| SpeedSegment {
            span,
            speed: Speed::from_thousandths(thousandths).unwrap(),
        })
        .collect()
}

fn ramp_item(source: i64, start: i64, segments: &[SpeedSegment]) -> Item {
    let curve = SpeedCurve::from_segments(Tick(source), segments).unwrap();
    testkit::item::item(
        ItemId(1),
        ItemParams::Video(VideoParams {
            source_audio_enabled: true,
            ..VideoParams::plain()
        }),
    )
    .asset(media::AssetId(3))
    .source(range(0, source))
    .start(Tick(start))
    .retime(Retime {
        window: None,
        speed: Speed::UNITY,
        maintain_pitch: true,
        reverse: false,
        curve: Some(curve),
    })
    .build()
}

#[test]
fn every_preset_builds_a_valid_curve_and_slow_in_fast_out_ramps_up() {
    for preset in SpeedPreset::ALL {
        let curve =
            SpeedCurve::from_segments(Tick(10 * TICKS_PER_SECOND), &preset.segments()).unwrap();
        assert!(curve.keys().len() >= 3, "{preset:?}");
        // The editable stretches come back with the spans and speeds they were built from.
        assert_eq!(curve.segments(), preset.segments(), "{preset:?}");
    }
    let second = TICKS_PER_SECOND;
    let item = ramp_item(10 * second, 0, &SpeedPreset::SlowInFastOut.segments());
    // First stretch: 2.5 s of source at 0.5x takes 5 s; the last runs at 2x.
    assert_eq!(item.source_time_at(Tick(second)).unwrap(), Tick(second / 2));
    let curve = item.retime.curve.unwrap();
    let second_key = curve.keys().get(1).unwrap();
    assert_eq!(second_key.output, Tick(5 * second));
    assert_eq!(second_key.source, Tick(5 * second / 2));
    let last = item.end().unwrap();
    let before = item.source_time_at(Tick(last.0 - second)).unwrap();
    let after = item.source_time_at(Tick(last.0 - second / 2)).unwrap();
    assert_eq!(after.0 - before.0, second, "2x in the last stretch");
}

#[test]
fn curve_to_source_time_is_monotonic_and_exact_at_every_key() {
    let second = TICKS_PER_SECOND;
    for preset in SpeedPreset::ALL {
        let item = ramp_item(10 * second, 3 * second, &preset.segments());
        let curve = item.retime.curve.unwrap();
        for key in curve.keys() {
            let at = Tick(item.start.0 + key.output.0);
            if at < item.end().unwrap() {
                assert_eq!(item.source_time_at(at).unwrap(), key.source, "{preset:?}");
            }
            assert_eq!(
                curve
                    .source_offset(key.output, time::Rounding::Floor)
                    .unwrap(),
                key.source
            );
            assert_eq!(
                curve
                    .output_offset(key.source, time::Rounding::Ceil)
                    .unwrap(),
                key.output
            );
        }
        assert_eq!(item.duration().unwrap(), curve.total_output());
        assert_eq!(item.source_time_at(item.start).unwrap(), Tick::ZERO);
        let last = item
            .source_time_at(Tick(item.end().unwrap().0 - 1))
            .unwrap();
        assert!(
            last < Tick(10 * second) && last.0 > 5 * second,
            "{preset:?}"
        );
        let mut previous = Tick::ZERO;
        let step = item.duration().unwrap().0 / 997;
        for n in 0..997 {
            let source = item.source_time_at(Tick(item.start.0 + n * step)).unwrap();
            assert!(source >= previous, "{preset:?} step {n}");
            previous = source;
        }
        // The inverse is the first output tick whose source is at or past the request.
        let probe = Tick(7 * second);
        let output = item.output_time_of(probe).unwrap();
        assert!(item.source_time_at(output).unwrap() >= probe);
        assert!(item.source_time_at(Tick(output.0 - 1)).unwrap() < probe);
    }
}

#[test]
fn split_and_trim_of_a_ramped_clip_reproduce_the_original_mapping() {
    let second = TICKS_PER_SECOND;
    for preset in SpeedPreset::ALL {
        let item = ramp_item(10 * second, second, &preset.segments());
        let length = item.duration().unwrap().0;
        for numerator in [1, 3, 5, 8, 9] {
            let at = Tick(item.start.0 + length * numerator / 10);
            check_split(&item, at);
            let (left, right) = item.split_at(at).unwrap();
            assert!(left.retime.curve.is_some() && right.retime.curve.is_some());
            // A second cut inside the right half still reproduces the original.
            let right_item = Item {
                source_range: right.source_range,
                start: right.start,
                retime: right.retime,
                ..item.clone()
            };
            let inner = Tick(right.start.0 + right_item.duration().unwrap().0 / 2);
            check_split(&right_item, inner);
            for t in [right.start.0, inner.0, item.end().unwrap().0 - 1] {
                assert_eq!(
                    right_item.source_time_at(Tick(t)).unwrap(),
                    item.source_time_at(Tick(t)).unwrap(),
                    "{preset:?} cut {numerator}"
                );
            }
        }
        // Trims inside keep the mapping; the opposite edge stays put.
        let at = Tick(item.start.0 + length / 3);
        let cut = item.trim_start(at).unwrap();
        assert_eq!(cut.start, at);
        let trimmed = Item {
            source_range: cut.source_range,
            start: cut.start,
            retime: cut.retime,
            ..item.clone()
        };
        assert_eq!(trimmed.end().unwrap(), item.end().unwrap());
        for t in [at.0, at.0 + 12_345, item.end().unwrap().0 - 1] {
            assert_eq!(
                trimmed.source_time_at(Tick(t)).unwrap(),
                item.source_time_at(Tick(t)).unwrap()
            );
        }
        let at = Tick(item.start.0 + 2 * length / 3);
        let cut = item.trim_end(at).unwrap();
        let trimmed = Item {
            source_range: cut.source_range,
            retime: cut.retime,
            ..item.clone()
        };
        assert_eq!(trimmed.end().unwrap(), at);
        assert_eq!(
            trimmed.source_time_at(Tick(at.0 - 1)).unwrap(),
            item.source_time_at(Tick(at.0 - 1)).unwrap()
        );
        // Extending past the curve cannot be mapped and refuses; so does reversing.
        assert!(item.trim_end(Tick(item.end().unwrap().0 + 1)).is_err());
        assert!(item.trim_start(Tick(item.start.0 - 1)).is_err());
        assert!(item.clone().set_reversed(true).is_err());
    }
}

#[test]
fn ramped_clip_snaps_to_the_29_97_frame_grid() {
    let second = TICKS_PER_SECOND;
    let frame = second * 1001 / 30000;
    assert_eq!(frame * 30000, second * 1001, "29.97 frames are whole ticks");
    // Normal speed for the first 30%: every output frame shows its own source frame.
    let slow_middle = ramp_item(10 * second, 0, &SpeedPreset::SlowMiddle.segments());
    for n in 0..89 {
        let at = Tick(n * frame);
        assert_eq!(slow_middle.source_time_at(at).unwrap(), at, "frame {n}");
    }
    // 3x in the middle: source frames advance by at most three per output frame and
    // never go backwards.
    let fast_middle = ramp_item(10 * second, 0, &SpeedPreset::FastMiddle.segments());
    let frames = fast_middle.duration().unwrap().0 / frame;
    let mut previous = 0;
    let mut deltas = Vec::new();
    for n in 0..frames {
        let source = fast_middle.source_time_at(Tick(n * frame)).unwrap().0 / frame;
        assert!(source >= previous && source - previous <= 3, "frame {n}");
        deltas.push(source - previous);
        previous = source;
    }
    assert!(deltas.contains(&3) && deltas.contains(&1));
}

#[test]
fn curve_persists_and_invalid_curves_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let build = |item: Item| {
        let mut data = empty().into_data();
        data.assets.push(source(dir.path().join("clip.mp4")));
        data.scene_mut().timeline.main.items.push(item);
        Project::new(data)
    };
    let ramp = ramp_item(1000, 0, &SpeedPreset::SlowMiddle.segments());
    let project = build(ramp.clone()).unwrap();
    assert_eq!(project.duration(), Tick(1934));
    let bytes = encode(&project, false).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(
        text.contains(r#""curve":[{"output":0,"source":0},{"output":300,"source":300}"#),
        "{text}"
    );
    assert_eq!(decode(&bytes).unwrap().project, project);
    // A constant rate serializes without a curve field, as before.
    let constant = build(
        testkit::item::item(
            ItemId(1),
            ItemParams::Video(VideoParams {
                source_audio_enabled: true,
                ..VideoParams::plain()
            }),
        )
        .asset(media::AssetId(3))
        .source(range(0, 400))
        .build(),
    )
    .unwrap();
    let text = String::from_utf8(encode(&constant, false).unwrap()).unwrap();
    assert!(!text.contains(r#""curve":["#), "{text}");

    // Item-level refusals: wrong source span, reverse, or a constant rate beside a curve.
    let mut wrong_span = ramp.clone();
    wrong_span.source_range = range(0, 999);
    assert!(build(wrong_span).is_err());
    let mut reversed = ramp.clone();
    reversed.retime.reverse = true;
    assert!(build(reversed).is_err());
    let mut doubled = ramp;
    doubled.retime.speed = Speed::from_thousandths(2000).unwrap();
    assert!(build(doubled).is_err());

    // Curve-level refusals.
    let key = |output, source| SpeedKey {
        output: Tick(output),
        source: Tick(source),
    };
    assert!(SpeedCurve::new(&[key(0, 0)]).is_err(), "one key");
    assert!(SpeedCurve::new(&[key(1, 0), key(2, 2)]).is_err(), "origin");
    assert!(
        SpeedCurve::new(&[key(0, 0), key(10, 10), key(10, 20)]).is_err(),
        "flat output"
    );
    assert!(
        SpeedCurve::new(&[key(0, 0), key(10, 10), key(20, 10)]).is_err(),
        "flat source"
    );
    assert!(
        SpeedCurve::new(&[key(0, 0), key(10, 10), key(5, 20)]).is_err(),
        "backwards"
    );
    assert!(
        SpeedCurve::new(&[key(0, 0), key(10, 1010)]).is_err(),
        "above 100x"
    );
    assert!(
        SpeedCurve::new(&[key(0, 0), key(10, 1000)]).is_ok(),
        "exactly 100x"
    );
    assert!(
        SpeedCurve::new(&[key(0, 0), key(1000, 99)]).is_err(),
        "below 0.1x"
    );
    assert!(
        SpeedCurve::new(&[key(0, 0), key(1000, 100)]).is_ok(),
        "exactly 0.1x"
    );
    let nine: Vec<_> = (0..9).map(|n| key(n * 10, n * 10)).collect();
    assert!(SpeedCurve::new(&nine).is_err(), "nine keys");
    assert!(
        SpeedCurve::new(nine.get(..8).unwrap()).is_ok(),
        "eight keys"
    );
    assert!(
        serde_json::from_str::<SpeedCurve>(
            r#"[{"output":0,"source":0},{"output":10,"source":1010}]"#
        )
        .is_err()
    );
    let wire = r#"[{"output":0,"source":0},{"output":10,"source":20}]"#;
    let good = serde_json::from_str::<SpeedCurve>(wire).unwrap();
    assert_eq!(serde_json::to_string(&good).unwrap(), wire);
    // Edited stretches: spans must sum to 1000 and each needs a source tick.
    assert!(SpeedCurve::from_segments(Tick(1000), &segments(&[(1000, 2000)])).is_ok());
    assert!(SpeedCurve::from_segments(Tick(1000), &segments(&[(1000, 100_000)])).is_ok());
    assert!(
        SpeedCurve::from_segments(Tick(1000), &segments(&[(500, 100_000), (500, 100)])).is_ok()
    );
    assert!(SpeedCurve::from_segments(Tick(1000), &segments(&[(500, 1000)])).is_err());
    assert!(SpeedCurve::from_segments(Tick(1000), &[]).is_err());
    assert!(SpeedCurve::from_segments(Tick(1), &segments(&[(500, 1000), (500, 1000)])).is_err());
    assert!(
        SpeedCurve::from_segments(Tick(1000), &segments(&[(125, 1000); 8])).is_err(),
        "seven stretches at most"
    );
}
