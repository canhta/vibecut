#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use crate::project::{empty, range, source};
use project::*;
use proptest::prelude::*;
use time::{Speed, TICKS_PER_SECOND, Tick};

const SPEEDS: [u16; 5] = [10, 500, 1000, 2000, 5000];

fn media(source_range: time::SourceRange, thousandths: u16, start: i64) -> Item {
    Item {
        blend_mode: Default::default(),
        retime: Retime {
            speed: Speed::from_thousandths(thousandths).unwrap(),
            maintain_pitch: false,
        },
        masks: Vec::new(),
        effects: Vec::new(),
        animations: Default::default(),
        id: ItemId(1),
        source_asset: Some(media::AssetId(3)),
        source_range,
        start: Tick(start),
        content: ItemParams::Video(VideoParams {
            fades: Default::default(),
            source_audio_enabled: true,
            volume: 0.0,
            muted: false,
        }),
    }
}

#[test]
fn rate_duration_and_source_range_agree_at_every_boundary_speed() {
    let second = TICKS_PER_SECOND;
    for thousandths in SPEEDS {
        let item = media(range(7 * second, 17 * second), thousandths, 3 * second);
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
                (back.0 - 7 * second - offset).abs() <= 5,
                "{speed} {offset}"
            );
        }
    }
}

fn check_split(item: &Item, at: Tick) {
    let (left, right) = item.split_at(at).unwrap();
    let speed = item.retime.speed;
    let (num, den) = speed.ratio();
    let left_end = left.start + speed.output_duration(left.source_range.duration()).unwrap();
    assert_eq!(left.start, item.start);
    assert_eq!(right.start, left_end, "pieces touch exactly");
    assert!(left_end <= at, "left edge stays at or before the request");
    assert!(
        at.0 - left_end.0 <= den / num + 1,
        "edge snaps by under one source tick"
    );
    assert_eq!(left.source_range.start(), item.source_range.start());
    assert_eq!(left.source_range.end(), right.source_range.start());
    assert_eq!(right.source_range.end(), item.source_range.end());
    let right_end = right.start
        + speed
            .output_duration(right.source_range.duration())
            .unwrap();
    assert!(
        right_end <= item.end().unwrap(),
        "no overlap with a neighbour"
    );
    assert!(
        item.end().unwrap().0 - right_end.0 <= 1,
        "at most the floor of one tick is lost"
    );
}

#[test]
fn split_and_trim_respect_retime_at_boundary_and_odd_cuts() {
    let second = TICKS_PER_SECOND;
    let frame = second / 30;
    for thousandths in SPEEDS {
        let item = media(range(0, 4 * second), thousandths, 10 * second);
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
                    start: cut.start,
                    source_range: cut.source_range,
                    ..item.clone()
                };
                assert_eq!(trimmed.end().unwrap(), end, "the end edge never moves");
            }
            if let Ok(cut) = item.trim_end(at) {
                let trimmed = Item {
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
    let unity = media(range(100, 200), 1000, 5);
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
        thousandths in 10u16..=5000,
        source_ticks in 20i64..4_000_000_000_000,
        cut in 1i64..4_000_000_000_000,
        start in 0i64..1_000_000_000,
    ) {
        let item = media(range(0, source_ticks), thousandths, start);
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
    let mut video = media(range(0, 400), 500, 0);
    video.retime.maintain_pitch = true;
    data.scene_mut().timeline.main.items.push(video);
    let mut audio = media(range(0, 300), 2500, 0);
    audio.id = ItemId(2);
    audio.content = ItemParams::Audio(AudioParams {
        fades: Default::default(),
        volume: 0.0,
        muted: false,
        source: Default::default(),
    });
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
        text.contains(r#""retime":{"speed":0.5,"maintain_pitch":true}"#),
        "{text}"
    );
    assert_eq!(decode(&bytes).unwrap().project, project);

    let mut text_item = media(range(0, 10), 1000, 0);
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
            speed: Speed::from_thousandths(speed).unwrap(),
            maintain_pitch: pitch,
        };
        assert!(
            Project::new(candidate).is_err(),
            "non-media item must stay at 1x"
        );
    }
    // 4 source ticks at 5x play for less than one output tick.
    let mut data = empty().into_data();
    data.assets.push(source(dir.path().join("clip.mp4")));
    data.scene_mut()
        .timeline
        .main
        .items
        .push(media(range(0, 4), 5000, 0));
    assert!(Project::new(data).is_err());
}

#[test]
fn long_retime_mapping_keeps_audio_and_video_within_one_sample_grid() {
    use time::Rounding;
    for thousandths in [10, 333, 500, 1000, 2000, 5000] {
        let item = media(
            range(7 * TICKS_PER_SECOND + 123, 7200 * TICKS_PER_SECOND),
            thousandths,
            3 * TICKS_PER_SECOND + 456,
        );
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
