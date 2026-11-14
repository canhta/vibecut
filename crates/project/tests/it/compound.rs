#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use project::*;
use time::{SourceRange, TICKS_PER_SECOND, Tick};

fn seconds(value: i64) -> Tick {
    Tick(value * TICKS_PER_SECOND)
}

fn overlay(id: u64, start: i64, length: i64) -> Item {
    testkit::item::item(
        ItemId(id),
        ItemParams::Overlay(OverlayParams {
            color: Color([200, 40, 40, 255]),
        }),
    )
    .source(SourceRange::new(Tick::ZERO, seconds(length)).unwrap())
    .start(seconds(start))
    .build()
}

fn track(id: u64, kind: TrackKind, items: Vec<Item>) -> Track {
    Track {
        id: TrackId(id),
        name: format!("Track {id}").into(),
        kind,
        muted: false,
        hidden: false,
        locked: false,
        items,
    }
}

/// Nested main track 10 holds overlay 20 at 0-1 s; nested overlay track 11 holds overlay 21 at
/// 1-2 s.
fn nested() -> Timeline {
    Timeline {
        transitions: Vec::new(),
        overlays: vec![track(11, TrackKind::Video, vec![overlay(21, 1, 1)])],
        main: track(10, TrackKind::Video, vec![overlay(20, 0, 1)]),
        magnet: false,
        audio: Vec::new(),
    }
}

/// Main track 0 holds overlay 1 at 0-2 s and compound clip 30 at 2-4 s.
fn with_compound(compound: Item) -> ProjectData {
    let mut data = Project::empty(ProjectId(7), "Compound".into())
        .unwrap()
        .into_data();
    data.scene_mut().timeline.main.items = vec![overlay(1, 0, 2), compound];
    data
}

fn compound() -> Item {
    Item::compound(ItemId(30), seconds(2), nested()).unwrap()
}

#[test]
fn compound_clip_round_trips_with_its_members() {
    let project = Project::new(with_compound(compound())).unwrap();
    assert_eq!(
        decode(&encode(&project, false).unwrap()).unwrap().project,
        project
    );
    assert_eq!(project.duration(), seconds(4));
    let ItemParams::Compound(params) = &project.data().scene().timeline.main.items[1].content
    else {
        panic!("compound clip");
    };
    assert_eq!(params.timeline, nested());
    assert_eq!(params.clip, CompoundParams::plain_clip());
}

#[test]
fn release_puts_members_on_their_tracks_at_output_time() {
    let mut data = with_compound(compound());
    release_compound(&mut data.scene_mut().timeline, ItemId(30)).unwrap();
    let project = Project::new(data).unwrap();
    let timeline = &project.data().scene().timeline;
    // The nested main track joins the host main track; the nested overlay sits above it.
    let main: Vec<_> = timeline
        .main
        .items
        .iter()
        .map(|item| (item.id, item.start))
        .collect();
    assert_eq!(main, [(ItemId(1), Tick::ZERO), (ItemId(20), seconds(2))]);
    let overlays: Vec<_> = timeline.overlays.iter().map(|track| track.id).collect();
    assert_eq!(overlays, [TrackId(11)]);
    assert_eq!(timeline.overlays[0].items[0].start, seconds(3));
}

#[test]
fn own_edits_are_a_window_retime_look_or_sound_of_the_clip_itself() {
    assert!(!compound().compound_has_own_edits().unwrap());
    assert!(!overlay(1, 0, 1).compound_has_own_edits().unwrap());
    let edits: Vec<(&str, Box<dyn Fn(&mut Item)>)> = vec![
        (
            "trimmed",
            Box::new(|item| {
                item.source_range = SourceRange::new(seconds(1), seconds(2)).unwrap();
            }),
        ),
        (
            "2x",
            Box::new(|item| {
                item.retime.speed = time::Speed::from_f64(2.0).unwrap();
            }),
        ),
        (
            "blend",
            Box::new(|item| item.blend_mode = BlendMode::Multiply),
        ),
        (
            "volume",
            Box::new(|item| {
                if let ItemParams::Compound(params) = &mut item.content {
                    params.clip.volume = -6.0;
                }
            }),
        ),
    ];
    for (case, edit) in edits {
        let mut item = compound();
        edit(&mut item);
        assert!(item.compound_has_own_edits().unwrap(), "{case}");
    }
}

#[test]
fn release_hides_members_of_a_hidden_compound_and_keeps_a_hidden_host_state() {
    let mut hidden = compound();
    hidden.hidden = true;
    let mut data = with_compound(hidden);
    release_compound(&mut data.scene_mut().timeline, ItemId(30)).unwrap();
    let project = Project::new(data).unwrap();
    let timeline = &project.data().scene().timeline;
    assert!(
        timeline
            .iter()
            .flat_map(|track| &track.items)
            .filter(|item| matches!(item.id, ItemId(20) | ItemId(21)))
            .all(|item| item.hidden)
    );

    // A hidden host track passes its state to the tracks it releases.
    let mut data = with_compound(compound());
    data.scene_mut().timeline.main.hidden = true;
    release_compound(&mut data.scene_mut().timeline, ItemId(30)).unwrap();
    let timeline = &data.scene().timeline;
    assert_eq!(timeline.overlays.len(), 1);
    assert!(timeline.overlays.iter().all(|track| track.hidden));
    assert!(timeline.main.items.iter().any(|item| item.id == ItemId(20)));
}

#[test]
fn release_refuses_an_item_that_is_not_a_compound_clip() {
    let mut data = with_compound(compound());
    assert!(release_compound(&mut data.scene_mut().timeline, ItemId(1)).is_err());
    assert!(release_compound(&mut data.scene_mut().timeline, ItemId(99)).is_err());
}

#[test]
fn renumber_gives_every_nested_identity_a_fresh_id() {
    let ItemParams::Compound(mut params) = compound().content else {
        panic!("compound clip");
    };
    let mut next = 100_u64;
    params
        .renumber(&mut || -> Result<u64> {
            next += 1;
            Ok(next)
        })
        .unwrap();
    let tracks = params.timeline.all_tracks();
    assert!(tracks.iter().all(|track| track.id.0 > 100));
    assert!(
        tracks
            .iter()
            .flat_map(|track| &track.items)
            .all(|item| item.id.0 > 100)
    );
}

#[test]
fn validation_refuses_invalid_compound_clips() {
    let deep = (0..=MAX_COMPOUND_DEPTH).fold(nested(), |inner, level| {
        let base = 1000 + 10 * level as u64;
        Timeline {
            transitions: Vec::new(),
            overlays: Vec::new(),
            main: track(
                base,
                TrackKind::Video,
                vec![Item::compound(ItemId(base + 1), Tick::ZERO, inner).unwrap()],
            ),
            magnet: false,
            audio: Vec::new(),
        }
    });
    let cases: Vec<(&str, Box<dyn Fn(&mut ProjectData)>)> = vec![
        (
            "member id used outside",
            Box::new(|data| data.scene_mut().timeline.main.items[0].id = ItemId(20)),
        ),
        (
            "nested track id used outside",
            Box::new(|data| data.scene_mut().timeline.main.id = TrackId(11)),
        ),
        (
            "window past the content",
            Box::new(|data| {
                data.scene_mut().timeline.main.items[1].source_range =
                    SourceRange::new(Tick::ZERO, seconds(3)).unwrap();
            }),
        ),
        (
            "source analysis",
            Box::new(|data| {
                if let ItemParams::Compound(params) =
                    &mut data.scene_mut().timeline.main.items[1].content
                {
                    params.clip.remove_flicker = true;
                }
            }),
        ),
        (
            "group inside",
            Box::new(|data| {
                if let ItemParams::Compound(params) =
                    &mut data.scene_mut().timeline.main.items[1].content
                {
                    params.timeline.main.items[0].group = Some(GroupId(77));
                }
            }),
        ),
        (
            "audio track kind",
            Box::new(|data| {
                let item = data.scene_mut().timeline.main.items.remove(1);
                data.scene_mut()
                    .timeline
                    .audio
                    .push(track(40, TrackKind::Audio, vec![item]));
            }),
        ),
    ];
    for (case, mutate) in cases {
        let mut data = with_compound(compound());
        mutate(&mut data);
        assert!(Project::new(data).is_err(), "{case}");
    }
    // A clip edit is valid: the second half of the content at 2x, multiplied, quieter.
    let mut edited = compound();
    edited.source_range = SourceRange::new(seconds(1), seconds(2)).unwrap();
    edited.retime.speed = time::Speed::from_f64(2.0).unwrap();
    edited.blend_mode = BlendMode::Multiply;
    if let ItemParams::Compound(params) = &mut edited.content {
        params.clip.volume = -6.0;
    }
    assert!(Project::new(with_compound(edited)).is_ok());
    let mut data = with_compound(compound());
    data.scene_mut().timeline.main.items[1] = Item::compound(ItemId(30), seconds(2), deep).unwrap();
    assert!(
        matches!(Project::new(data), Err(Error::TooLarge(_))),
        "nesting deeper than the bound"
    );
    let empty = Timeline {
        magnet: false,
        ..Timeline::default()
    };
    assert!(Item::compound(ItemId(30), Tick::ZERO, empty).is_err());
}
