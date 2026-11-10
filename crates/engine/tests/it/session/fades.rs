#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use engine::{Command, Session};
use media::{AssetId, ContentHash};
use project::{
    AudioParams, Fade, FadeCurve, FadeEdge, Item, ItemId, ItemParams, Project, ProjectId, Track,
    TrackId, TrackKind,
};
use serde_json::json;
use time::{SourceRange, Tick};

/// Largest integer the JSON control boundary admits.
const MAX_JSON: i64 = (1 << 53) - 1;

fn session() -> Session {
    session_len(1000)
}
fn session_len(len: i64) -> Session {
    let mut data = Project::empty(ProjectId(0), "fades".into())
        .unwrap()
        .into_data();
    data.assets.push(
        testkit::source::asset(7, std::env::temp_dir().join("fades.wav"))
            .format("wav")
            .duration(Tick(len))
            .content_hash(ContentHash::from_bytes(&[7]))
            .stream(
                testkit::source::audio()
                    .codec("pcm_f32le")
                    .time_base(media::TimeBase::new(1, 48000).unwrap())
                    .duration(Tick(len))
                    .build(),
            )
            .build(),
    );
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(8),
        name: "".into(),
        kind: TrackKind::Audio,
        hidden: false,
        muted: false,
        locked: false,
        items: vec![Item {
            hidden: false,
            retime: Default::default(),
            id: ItemId(20),
            source_asset: Some(AssetId(7)),
            source_range: SourceRange::new(Tick::ZERO, Tick(len)).unwrap(),
            start: Tick::ZERO,
            animations: Default::default(),
            effects: vec![],
            masks: vec![],
            blend_mode: Default::default(),
            content: ItemParams::Audio(AudioParams {
                fades: Default::default(),
                volume: 0.0,
                muted: false,
                source: Default::default(),
            }),
        }],
    });
    Session::new(Project::new(data).unwrap(), 100).unwrap()
}
fn fades(session: &Session, id: u64) -> project::Fades {
    session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find_map(|i| match &i.content {
            ItemParams::Audio(AudioParams { fades, .. }) if i.id == ItemId(id) => Some(*fades),
            _ => None,
        })
        .unwrap()
}
fn set(
    session: &mut Session,
    edge: FadeEdge,
    duration: i64,
    curve: FadeCurve,
) -> engine::Result<()> {
    session
        .apply(Command::SetFade {
            item: ItemId(20),
            edge,
            duration: Tick(duration),
            curve,
        })
        .map(|_| ())
}

#[test]
fn set_fade_is_undoable_redoable_and_persisted() {
    let mut s = session();
    assert_eq!(fades(&s, 20), project::Fades::default());
    for (edge, duration, curve) in [
        (FadeEdge::In, 300, FadeCurve::Linear),
        (FadeEdge::Out, 200, FadeCurve::Exponential),
    ] {
        let before = s.project().clone();
        set(&mut s, edge, duration, curve).unwrap();
        let after = s.project().clone();
        assert_ne!(before, after);
        s.undo().unwrap();
        assert_eq!(&before, s.project());
        s.redo().unwrap();
        assert_eq!(&after, s.project());
    }
    let expected = project::Fades {
        fade_in: Fade {
            duration: Tick(300),
            curve: FadeCurve::Linear,
            ..Fade::default()
        },
        fade_out: Fade {
            duration: Tick(200),
            curve: FadeCurve::Exponential,
            ..Fade::default()
        },
    };
    assert_eq!(fades(&s, 20), expected);
    let encoded = project::encode(s.project(), false).unwrap();
    let loaded = project::decode(&encoded).unwrap().project;
    assert_eq!(&loaded, s.project().as_ref());
}

#[test]
fn set_fade_clamps_to_clip_without_overlapping_the_other_edge() {
    let mut s = session();
    set(&mut s, FadeEdge::Out, 700, FadeCurve::EqualPower).unwrap();
    set(&mut s, FadeEdge::In, 5000, FadeCurve::EqualPower).unwrap();
    assert_eq!(fades(&s, 20).fade_in.duration, Tick(300));
    assert_eq!(fades(&s, 20).fade_out.duration, Tick(700));
    set(&mut s, FadeEdge::Out, MAX_JSON, FadeCurve::Linear).unwrap();
    assert_eq!(fades(&s, 20).fade_out.duration, Tick(700));
    set(&mut s, FadeEdge::In, 0, FadeCurve::Linear).unwrap();
    set(&mut s, FadeEdge::Out, MAX_JSON, FadeCurve::Linear).unwrap();
    assert_eq!(fades(&s, 20).fade_out.duration, Tick(1000));
}

#[test]
fn set_fade_refuses_hostile_input_atomically() {
    let mut s = session();
    let before = s.project().clone();
    let history = s.history_len();
    assert!(set(&mut s, FadeEdge::In, -1, FadeCurve::Linear).is_err());
    assert!(set(&mut s, FadeEdge::In, i64::MIN, FadeCurve::Linear).is_err());
    assert!(
        crate::wire::run(
            &mut s,
            "audio.set_fade",
            json!({"item":20,"edge":"in","duration":10,"curve":"cubic"})
        )
        .is_err()
    );
    assert!(
        crate::wire::run(
            &mut s,
            "audio.set_fade",
            json!({"item":20,"edge":"middle","duration":10,"curve":"linear"})
        )
        .is_err()
    );
    assert!(
        s.apply(Command::SetFade {
            item: ItemId(999),
            edge: FadeEdge::In,
            duration: Tick(1),
            curve: FadeCurve::Linear,
        })
        .is_err()
    );
    assert!(
        crate::wire::run(
            &mut s,
            "audio.set_fade",
            json!({"item":20,"edge":"in","duration":1,"curve":"linear","extra":1})
        )
        .is_err()
    );
    assert!(crate::wire::run(&mut s, "audio.set_fade", json!({"item":20,"edge":"in"})).is_err());
    assert_eq!(s.project(), &before);
    assert_eq!(s.history_len(), history);
    s.apply(Command::SetTrackLocked {
        track: TrackId(8),
        locked: true,
    })
    .unwrap();
    let locked = s.project().clone();
    assert!(set(&mut s, FadeEdge::In, 10, FadeCurve::Linear).is_err());
    assert_eq!(s.project(), &locked);
}

/// One timeline piece: id, start and end ticks, fades.
type Piece = (u64, Tick, Tick, project::Fades);
fn pieces(session: &Session) -> Vec<Piece> {
    let mut all: Vec<Piece> = session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .filter_map(|i| match &i.content {
            ItemParams::Audio(AudioParams { fades, .. }) => {
                Some((i.id.0, i.start, i.end().unwrap(), *fades))
            }
            _ => None,
        })
        .collect();
    all.sort_by_key(|p| p.1);
    all
}
/// Gain of every output sample the pieces cover, indexed by absolute sample.
fn envelope(session: &Session, rate: i64) -> Vec<f32> {
    let mut out = Vec::new();
    let all = pieces(session);
    let base = all
        .first()
        .unwrap()
        .1
        .to_units(rate, time::Rounding::Nearest)
        .unwrap();
    for (_, start, end, fades) in all {
        let spans = fades.spans(start, end, rate).unwrap();
        let (i, o) = spans.lengths();
        let samples = end.to_units(rate, time::Rounding::Nearest).unwrap()
            - start.to_units(rate, time::Rounding::Nearest).unwrap();
        assert!(i + o <= samples, "fades overlap on piece {start:?}");
        let first = start.to_units(rate, time::Rounding::Nearest).unwrap();
        let last = end.to_units(rate, time::Rounding::Nearest).unwrap();
        assert_eq!(
            (first - base) as usize,
            out.len(),
            "pieces must tile the clip"
        );
        out.extend((first..last).map(|n| spans.gain(n - first)));
    }
    out
}
fn fade_edges(session: &mut Session, curve_in: FadeCurve, curve_out: FadeCurve) {
    set(session, FadeEdge::In, 300, curve_in).unwrap();
    set(session, FadeEdge::Out, 400, curve_out).unwrap();
}
const CURVE_NAMES: [FadeCurve; 3] = [
    FadeCurve::EqualPower,
    FadeCurve::Linear,
    FadeCurve::Exponential,
];

#[test]
fn split_inside_a_fade_keeps_the_envelope_the_original_had() {
    // The reviewed reproduction: 1000 ticks, 300 linear in, 400 exponential out, one tick
    // per sample so the curve is checked at exact integer positions.
    let rate = time::TICKS_PER_SECOND;
    let mut s = session();
    fade_edges(&mut s, FadeCurve::Linear, FadeCurve::Exponential);
    let original = envelope(&s, rate);
    assert_eq!(original[75], 0.25);
    s.apply(Command::SplitAt {
        item: ItemId(20),
        at: Tick(150),
    })
    .unwrap();
    let cut = envelope(&s, rate);
    assert_eq!(cut[75], 0.25);
    assert_eq!(cut[150], 0.5);
    assert_eq!(original, cut);
    s.undo().unwrap();
    s.apply(Command::SplitAt {
        item: ItemId(20),
        at: Tick(800),
    })
    .unwrap();
    let cut = envelope(&s, rate);
    assert!((cut[799] - 0.11920292).abs() < 1e-6, "{}", cut[799]);
    assert_eq!(original, cut);
}

#[test]
fn every_cut_of_every_curve_pair_replays_the_original_envelope() {
    let rate = time::TICKS_PER_SECOND;
    for curve_in in CURVE_NAMES {
        for curve_out in CURVE_NAMES {
            let mut s = session();
            fade_edges(&mut s, curve_in, curve_out);
            let original = envelope(&s, rate);
            // Inside the fade-in, at its end, in the body, at the fade-out start, inside it,
            // and one tick from either clip edge.
            for at in [1, 150, 299, 300, 301, 500, 599, 600, 601, 800, 999] {
                s.apply(Command::SplitAt {
                    item: ItemId(20),
                    at: Tick(at),
                })
                .unwrap();
                assert_eq!(
                    original,
                    envelope(&s, rate),
                    "{curve_in:?}/{curve_out:?} cut {at}"
                );
                s.undo().unwrap();
                assert_eq!(original, envelope(&s, rate));
            }
        }
    }
}

#[test]
fn a_cut_at_a_fade_boundary_gives_each_piece_only_its_own_fade() {
    let mut s = session();
    fade_edges(&mut s, FadeCurve::Linear, FadeCurve::Exponential);
    s.apply(Command::SplitAt {
        item: ItemId(20),
        at: Tick(300),
    })
    .unwrap();
    let ps = pieces(&s);
    let [(_, _, _, left), (_, _, _, right)] = ps[..] else {
        panic!("two pieces")
    };
    assert_eq!(left.fade_in.window(), Tick(300));
    assert_eq!(left.fade_out, Fade::default());
    assert_eq!(right.fade_in, Fade::default());
    assert_eq!(right.fade_out.window(), Tick(400));
    s.undo().unwrap();
    s.apply(Command::SplitAt {
        item: ItemId(20),
        at: Tick(600),
    })
    .unwrap();
    let ps = pieces(&s);
    let [(_, _, _, left), (_, _, _, right)] = ps[..] else {
        panic!("two pieces")
    };
    assert_eq!(left.fade_out, Fade::default());
    assert_eq!(right.fade_in, Fade::default());
    assert_eq!(right.fade_out.window(), Tick(400));
}

#[test]
fn a_split_keeps_the_whole_fade_and_the_consumed_part() {
    let mut s = session();
    fade_edges(&mut s, FadeCurve::Linear, FadeCurve::Exponential);
    s.apply(Command::SplitAt {
        item: ItemId(20),
        at: Tick(150),
    })
    .unwrap();
    let ps = pieces(&s);
    let [(_, _, _, left), (_, _, _, right)] = ps[..] else {
        panic!("two pieces")
    };
    assert_eq!(
        (left.fade_in.duration, left.fade_in.offset),
        (Tick(300), Tick::ZERO)
    );
    assert_eq!(
        (right.fade_in.duration, right.fade_in.offset),
        (Tick(300), Tick(150))
    );
    assert_eq!(right.fade_in.curve, FadeCurve::Linear);
    assert_eq!(left.fade_out, Fade::default());
    assert_eq!(right.fade_out.window(), Tick(400));
}

#[test]
fn trim_then_split_and_split_twice_stay_non_destructive() {
    let rate = time::TICKS_PER_SECOND;
    let mut s = session();
    fade_edges(&mut s, FadeCurve::EqualPower, FadeCurve::Linear);
    // Trimming restarts the fades at the new edges (the clip is now 100..900).
    s.apply(Command::TrimStart {
        item: ItemId(20),
        at: Tick(100),
    })
    .unwrap();
    s.apply(Command::TrimEnd {
        item: ItemId(20),
        at: Tick(900),
    })
    .unwrap();
    let trimmed = envelope(&s, rate);
    for at in [150, 399, 500, 880] {
        s.apply(Command::SplitAt {
            item: ItemId(20),
            at: Tick(at),
        })
        .unwrap();
        assert_eq!(trimmed, envelope(&s, rate), "trim then cut {at}");
        s.undo().unwrap();
    }
    // Split twice: cut inside the fade-in, then cut each half again inside the fades.
    s.apply(Command::SplitAt {
        item: ItemId(20),
        at: Tick(250),
    })
    .unwrap();
    s.apply(Command::SplitAt {
        item: ItemId(20),
        at: Tick(150),
    })
    .unwrap();
    for (piece, at) in [(250, 700), (700, 850)] {
        let id = pieces(&s)
            .into_iter()
            .find(|p| p.1 == Tick(piece))
            .unwrap()
            .0;
        s.apply(Command::SplitAt {
            item: ItemId(id),
            at: Tick(at),
        })
        .unwrap();
    }
    assert_eq!(pieces(&s).len(), 5);
    assert_eq!(trimmed, envelope(&s, rate));
}

#[test]
fn split_pieces_undo_redo_and_persist_with_their_envelope() {
    let rate = time::TICKS_PER_SECOND;
    let mut s = session();
    fade_edges(&mut s, FadeCurve::Exponential, FadeCurve::EqualPower);
    let original = envelope(&s, rate);
    let before = s.project().clone();
    s.apply(Command::SplitAt {
        item: ItemId(20),
        at: Tick(150),
    })
    .unwrap();
    s.apply(Command::SplitAt {
        item: ItemId(20),
        at: Tick(75),
    })
    .unwrap();
    let after = s.project().clone();
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(&before, s.project());
    assert_eq!(original, envelope(&s, rate));
    s.redo().unwrap();
    s.redo().unwrap();
    assert_eq!(&after, s.project());
    assert_eq!(original, envelope(&s, rate));
    let encoded = project::encode(s.project(), false).unwrap();
    let loaded = project::decode(&encoded).unwrap().project;
    assert_eq!(&loaded, s.project().as_ref());
    let reloaded = Session::new(loaded, 100).unwrap();
    assert_eq!(original, envelope(&reloaded, rate));
}

#[test]
fn off_grid_cuts_at_real_sample_rates_replay_the_original_samples() {
    // A 100 ms clip with fades and cuts that fall between samples at 44.1 and 48 kHz.
    let len = time::TICKS_PER_SECOND / 10 + 4321;
    for rate in [44_100, 48_000] {
        let mut s = session_len(len);
        let ms = |n: i64| time::TICKS_PER_SECOND / 1000 * n + 17;
        for (curve_in, curve_out) in CURVE_NAMES
            .into_iter()
            .zip(CURVE_NAMES.into_iter().cycle().skip(1))
        {
            set(&mut s, FadeEdge::In, ms(30), curve_in).unwrap();
            set(&mut s, FadeEdge::Out, ms(30), curve_out).unwrap();
            let original = envelope(&s, rate);
            for at in [ms(3), ms(30), ms(41), ms(72), ms(99)] {
                s.apply(Command::SplitAt {
                    item: ItemId(20),
                    at: Tick(at),
                })
                .unwrap();
                assert_eq!(original, envelope(&s, rate), "rate {rate} cut {at}");
                s.undo().unwrap();
            }
        }
    }
}

#[test]
fn a_trim_that_would_overlap_the_fades_shrinks_the_trimmed_edge() {
    let rate = time::TICKS_PER_SECOND;
    let mut s = session();
    fade_edges(&mut s, FadeCurve::EqualPower, FadeCurve::Linear);
    // Clip 100..500 is shorter than 300 + 400: the trimmed (out) edge yields.
    s.apply(Command::TrimStart {
        item: ItemId(20),
        at: Tick(100),
    })
    .unwrap();
    s.apply(Command::TrimEnd {
        item: ItemId(20),
        at: Tick(500),
    })
    .unwrap();
    let f = fades(&s, 20);
    assert_eq!(f.fade_in.window(), Tick(300));
    assert_eq!(f.fade_out.window(), Tick(100));
    let trimmed = envelope(&s, rate);
    for at in [200, 399, 450] {
        s.apply(Command::SplitAt {
            item: ItemId(20),
            at: Tick(at),
        })
        .unwrap();
        assert_eq!(trimmed, envelope(&s, rate), "cut {at}");
        s.undo().unwrap();
    }
    s.undo().unwrap();
    s.redo().unwrap();
    assert_eq!(fades(&s, 20), f);
}

#[test]
fn trimming_to_the_current_edge_preserves_a_split_fade() {
    for (cut, command) in [(150, "trim_start"), (800, "trim_end")] {
        let mut s = session();
        fade_edges(&mut s, FadeCurve::Linear, FadeCurve::Exponential);
        s.apply(Command::SplitAt {
            item: ItemId(20),
            at: Tick(cut),
        })
        .unwrap();
        let piece = pieces(&s)
            .into_iter()
            .find(|p| {
                if command == "trim_start" {
                    p.1 == Tick(cut)
                } else {
                    p.2 == Tick(cut)
                }
            })
            .unwrap();
        let before = s.project().clone();
        let original = envelope(&s, time::TICKS_PER_SECOND);
        let trim = |s: &mut Session, item: ItemId, at: i64| {
            if command == "trim_start" {
                s.apply(Command::TrimStart { item, at: Tick(at) })
            } else {
                s.apply(Command::TrimEnd { item, at: Tick(at) })
            }
        };
        trim(&mut s, ItemId(piece.0), cut).unwrap();
        let after = envelope(&s, time::TICKS_PER_SECOND);
        for (n, (original, after)) in original.iter().zip(&after).enumerate() {
            assert_eq!(original, after, "{command} cut {cut}, sample {n}");
        }
        assert_eq!(&before, s.project());
    }
}

#[test]
fn retime_fade_limits_and_split_use_output_time() {
    for (speed, clip) in [(0.5, 2000), (2.0, 500)] {
        let mut s = session();
        s.apply(Command::SetSpeed {
            items: vec![ItemId(20)],
            speed: time::Speed::from_f64(speed).unwrap(),
            maintain_pitch: true,
        })
        .unwrap();
        set(&mut s, FadeEdge::In, clip, FadeCurve::Linear).unwrap();
        let before = fades(&s, 20);
        assert_eq!(before.fade_in.duration, Tick(clip));
        let cut = clip / 4;
        let out = s
            .apply(Command::SplitAt {
                item: ItemId(20),
                at: Tick(cut),
            })
            .unwrap();
        let right_id = out
            .created
            .iter()
            .find_map(|id| match id {
                engine::CreatedId::Item(id) => Some(id.0),
                _ => None,
            })
            .unwrap();
        let left = fades(&s, 20);
        let right = fades(&s, right_id);
        assert_eq!(left.fade_in.duration, Tick(clip));
        assert_eq!(right.fade_in.duration, Tick(clip));
        assert_eq!(right.fade_in.offset, Tick(cut));
        let rate = time::TICKS_PER_SECOND;
        let whole = before.spans(Tick::ZERO, Tick(clip), rate).unwrap();
        let a = left.spans(Tick::ZERO, Tick(cut), rate).unwrap();
        let b = right.spans(Tick(cut), Tick(clip), rate).unwrap();
        for sample in 0..clip {
            assert_eq!(
                whole.gain(sample),
                if sample < cut {
                    a.gain(sample)
                } else {
                    b.gain(sample - cut)
                }
            );
        }
    }
}
