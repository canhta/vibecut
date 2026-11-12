#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use crate::support::session::{five_clip_pairs, project, range, session};
use engine::{Command, EditError, Session, control::OutputSpan};
use media::{AssetId, ContentHash};
use project::{Item, ItemId, ItemParams, Project, ProjectId, Track, TrackId, TrackKind};
use time::Tick;

#[test]
fn remove_pauses_cuts_reviewed_spans_as_one_undo_step() {
    let mut s = session();
    let before = project::encode(s.project(), false).unwrap();
    let revision = s.revision();
    let outcome = s
        .apply(Command::RemovePauses {
            spans: vec![OutputSpan {
                start: Tick(20),
                end: Tick(60),
            }],
            crossfade_ms: 0,
        })
        .unwrap();
    assert!(outcome.changed);
    assert_eq!(s.history_len(), (1, 0));
    assert_ne!(s.revision(), revision);
    let edited = project::encode(s.project(), false).unwrap();
    let track = &s.project().data().scene().timeline.overlays[0];
    let layout: Vec<(Tick, Tick)> = track
        .items
        .iter()
        .map(|item| (item.start, item.end().unwrap()))
        .collect();
    // The cut half-open span [20,60) leaves one left piece and shifts the follower.
    assert_eq!(layout, vec![(Tick(0), Tick(20)), (Tick(40), Tick(80))]);

    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), before);
    s.redo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), edited);
    let loaded = project::decode(&edited).unwrap().project;
    assert_eq!(&loaded, s.project().as_ref());

    // A nested batch child still cuts through the ripple path.
    let mut batched = session();
    batched
        .apply(Command::Batch {
            commands: vec![Command::RemovePauses {
                spans: vec![OutputSpan {
                    start: Tick(20),
                    end: Tick(60),
                }],
                crossfade_ms: 0,
            }],
        })
        .unwrap();
    assert_eq!(project::encode(batched.project(), false).unwrap(), edited);
}
#[test]
fn remove_pauses_refuses_invalid_reviewed_spans_atomically() {
    let mut s = session();
    let before = project::encode(s.project(), false).unwrap();
    let revision = s.revision();
    let span = |start, end| OutputSpan {
        start: Tick(start),
        end: Tick(end),
    };
    let mut cases = vec![
        vec![],
        vec![span(100, 60)],
        vec![span(20, 60), span(10, 15)],
        vec![span(10, 50), span(40, 60)],
        vec![span(20, 20)],
        vec![span(-1, 10)],
        vec![span(0, 1000)],
    ];
    let many: Vec<OutputSpan> = (0..=engine::control::MAX_PAUSE_SPANS)
        .map(|index| span(index as i64 * 2, index as i64 * 2 + 1))
        .collect();
    cases.push(many);
    for spans in cases {
        assert!(
            s.apply(Command::RemovePauses {
                spans,
                crossfade_ms: 0
            })
            .is_err()
        );
        assert_eq!(project::encode(s.project(), false).unwrap(), before);
        assert_eq!(s.revision(), revision);
        assert_eq!(s.history_len(), (0, 0));
    }
    assert!(
        s.apply(Command::RemovePauses {
            spans: vec![span(0, 10)],
            crossfade_ms: 10001
        })
        .is_err()
    );

    // A caption word forbids any span that crosses it.
    let mut data = project().into_data();
    data.assets.push(
        testkit::source::asset(7, std::env::temp_dir().join("pause-words.mp4"))
            .format("mp4")
            .duration(Tick(1000))
            .content_hash(ContentHash::from_bytes(b"words"))
            .build(),
    );
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(9),
        name: "Captions".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: ItemId(90),
            source_asset: Some(AssetId(7)),
            source_range: range(0, 1000),
            start: Tick::ZERO,
            content: ItemParams::Caption(project::CaptionParams {
                word_highlight: false,
                cues: captions::CueTrack {
                    styles: Vec::new(),
                    cues: vec![captions::Cue {
                        word_timing: captions::CueWordTiming::Unverified,
                        id: "cue-1".into(),
                        range: range(0, 1000),
                        source_lang: captions::LanguageTag::parse("en").unwrap(),
                        texts: std::collections::BTreeMap::new(),
                        word_ranges: vec![range(10, 20)],
                        style: None,
                    }],
                },
            }),
        }],
    });
    let mut words = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let before = project::encode(words.project(), false).unwrap();
    assert!(
        words
            .apply(Command::RemovePauses {
                spans: vec![span(5, 15)],
                crossfade_ms: 0
            })
            .is_err()
    );
    assert_eq!(project::encode(words.project(), false).unwrap(), before);
    // A span beside the word is allowed.
    assert!(
        words
            .apply(Command::RemovePauses {
                spans: vec![span(20, 30)],
                crossfade_ms: 0
            })
            .is_ok()
    );

    // A locked track refuses the whole edit.
    let mut locked = session();
    let track = locked.project().data().scene().timeline.overlays[0].id;
    locked
        .apply(Command::SetTrackLocked {
            track,
            locked: true,
        })
        .unwrap();
    let before = project::encode(locked.project(), false).unwrap();
    assert!(matches!(
        locked.apply(Command::RemovePauses {
            spans: vec![span(20, 60)],
            crossfade_ms: 0
        }),
        Err(EditError::Locked(_))
    ));
    assert_eq!(project::encode(locked.project(), false).unwrap(), before);

    // An empty timeline has no span to remove.
    let mut empty = Session::new(Project::empty(ProjectId(0), "".into()).unwrap(), 1).unwrap();
    assert!(
        empty
            .apply(Command::RemovePauses {
                spans: vec![span(0, 10)],
                crossfade_ms: 0
            })
            .is_err()
    );
}
#[test]
fn remove_pauses_keeps_five_clip_audio_video_aligned() {
    let mut s = five_clip_pairs();
    let before = project::encode(s.project(), false).unwrap();
    let spans = |seconds: &[(i64, i64)]| -> Vec<OutputSpan> {
        seconds
            .iter()
            .map(|(start, end)| OutputSpan {
                start: Tick(start * time::TICKS_PER_SECOND),
                end: Tick(end * time::TICKS_PER_SECOND),
            })
            .collect()
    };
    s.apply(Command::RemovePauses {
        spans: spans(&[(2, 3), (4, 5), (8, 10)]),
        crossfade_ms: 20,
    })
    .unwrap();
    let data = s.project().data().clone();
    let starts = |kind: TrackKind| -> Vec<Tick> {
        let mut starts: Vec<Tick> = data
            .scene()
            .timeline
            .iter()
            .filter(|track| track.kind == kind)
            .flat_map(|track| track.items.iter().map(|item| item.start))
            .collect();
        starts.sort_unstable();
        starts
    };
    assert_eq!(starts(TrackKind::Video), starts(TrackKind::Audio));
    assert_eq!(starts(TrackKind::Video).len(), 5);
    let saved = 4 * time::TICKS_PER_SECOND;
    assert_eq!(
        data.scene()
            .timeline
            .main
            .items
            .last()
            .unwrap()
            .end()
            .unwrap(),
        Tick(12 * time::TICKS_PER_SECOND - saved)
    );
    // Three joins per media track each carry one equal-power fade-in and fade-out.
    let (fade_ins, fade_outs) = data
        .scene()
        .timeline
        .iter()
        .filter(|track| track.kind == TrackKind::Audio)
        .flat_map(|track| track.items.iter())
        .fold((0, 0), |(ins, outs), item| match &item.content {
            ItemParams::Audio(params) => (
                ins + usize::from(params.fades.fade_in.window() > Tick::ZERO),
                outs + usize::from(params.fades.fade_out.window() > Tick::ZERO),
            ),
            _ => (ins, outs),
        });
    assert_eq!((fade_ins, fade_outs), (3, 3));
    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), before);
}
