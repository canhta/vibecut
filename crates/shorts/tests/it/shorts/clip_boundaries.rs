#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
use ai::clips::{ClipRole, ClipSourceSegment};
use captions::{
    Fingerprint, LanguageRegistry, LanguageTag, LayoutHooks, Segment, Transcript, Word,
};
use media::SpeechEnergyWindow;
use shorts::Error;
use shorts::clip_boundaries::*;
use time::{FrameRate, Rounding, SourceRange, Tick};

fn t(seconds: f64) -> Tick {
    Tick::from_seconds_f64(seconds, Rounding::Nearest).unwrap()
}
fn range(start: f64, end: f64) -> SourceRange {
    SourceRange::new(t(start), t(end)).unwrap()
}
fn transcript(words: bool) -> Transcript {
    Transcript {
        source_lang: LanguageTag::parse("en").unwrap(),
        fingerprint: Fingerprint {
            provider: "fixture".into(),
            model: "labelled".into(),
            settings: "known-pauses".into(),
        },
        segments: [
            (0.2, 1.6, "Hello world."),
            (2.4, 3.6, "Next."),
            (4.4, 5.6, "Done."),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (start, end, text))| Segment {
            id: format!("s{i}").into(),
            range: range(start, end),
            text: text.into(),
            words: if words && i == 0 {
                vec![word("Hello", 0.2, 0.8), word("world.", 0.9, 1.6)]
            } else {
                vec![]
            },
            spans: None,
        })
        .collect(),
    }
}
fn word(text: &str, start: f64, end: f64) -> Word {
    Word {
        text: text.into(),
        range: Some(range(start, end)),
        speaker: None,
        confidence: None,
    }
}
fn settings() -> ClipBoundarySettings {
    ClipBoundarySettings {
        source_range: range(0., 6.),
        frame_rate: FrameRate::FPS_30,
        tolerance: t(0.5),
        silence_rms: 0.001,
        min_duration: t(0.1),
        max_duration: t(6.),
        min_segment_duration: t(0.1),
        max_segments: 4,
    }
}
fn piece(start: f64, end: f64, role: ClipRole, source_order: usize) -> ClipSourceSegment {
    ClipSourceSegment {
        range: range(start, end),
        role,
        source_order,
        repair: None,
    }
}
fn quiet() -> Vec<SpeechEnergyWindow> {
    [(0., 0.2), (1.6, 2.4), (3.6, 4.4), (5.6, 6.)]
        .into_iter()
        .map(|(a, b)| SpeechEnergyWindow {
            range: range(a, b),
            rms: 0.0,
        })
        .collect()
}
fn snap(
    shorts: &[Vec<ClipSourceSegment>],
    transcript: &Transcript,
    energy: &[SpeechEnergyWindow],
    s: &ClipBoundarySettings,
) -> shorts::Result<Vec<SnappedShort>> {
    let registry = LanguageRegistry::standard().unwrap();
    snap_clip_boundaries(
        shorts,
        transcript,
        registry.profile(&transcript.source_lang),
        &LayoutHooks::default(),
        energy,
        s,
    )
}

#[test]
fn snaps_every_edge_preserving_reordered_roles_and_reports_deltas() {
    let candidates = vec![vec![
        piece(2.55, 3.5, ClipRole::Hook, 1),
        piece(0.4, 1.5, ClipRole::Payoff, 0),
    ]];
    let output = snap(&candidates, &transcript(true), &quiet(), &settings()).unwrap();
    let result = &output[0];
    assert_eq!(result.segments[0].role, ClipRole::Hook);
    assert_eq!(result.segments[0].source_order, 1);
    assert_eq!(result.segments[0].range, range(2.4, 3.6));
    assert_eq!(result.segments[1].range, range(0.2, 1.6));
    assert_eq!(result.duration, t(2.6));
    for (segment, adjustment) in result.segments.iter().zip(&result.adjustments) {
        assert_eq!(adjustment.start.adjusted, segment.range.start());
        assert_eq!(adjustment.end.adjusted, segment.range.end());
        for edge in [adjustment.start, adjustment.end] {
            assert_eq!(edge.delta, edge.adjusted - edge.original);
            assert!(edge.delta.abs() <= settings().tolerance);
            assert_eq!(
                settings().frame_rate.snap(edge.adjusted).unwrap(),
                edge.adjusted
            );
            assert_eq!(edge.kind, BoundaryKind::SentenceSilence);
        }
    }
    assert_eq!(candidates[0][0].range, range(2.55, 3.5));
}

#[test]
fn no_word_timing_protects_whole_segment_even_with_silent_energy() {
    let candidates = vec![vec![piece(0.65, 1.5, ClipRole::Payoff, 0)]];
    let mut s = settings();
    s.tolerance = t(0.2);
    assert!(matches!(
        snap(
            &candidates,
            &transcript(false),
            &[SpeechEnergyWindow {
                range: range(0., 6.),
                rms: 0.
            }],
            &s
        ),
        Err(Error::ClipBoundary {
            reason: BoundaryFailure::NoSafeFrame,
            ..
        })
    ));
    // Complete matching word timing permits the internal word gap.
    let out = snap(&candidates, &transcript(true), &[], &s).unwrap();
    assert_eq!(out[0].segments[0].range.start(), t(0.8));
    assert_eq!(
        out[0].adjustments[0].start.kind,
        BoundaryKind::WordOrSegment
    );
    for (first_end, second_start, expected) in [(0.8, 0.9, 0.8666666666666667), (0.811, 0.819, 0.8)]
    {
        let mut tr = transcript(true);
        tr.segments[0].words[0].range = Some(range(0.2, first_end));
        tr.segments[0].words[1].range = Some(range(second_start, 1.6));
        let energy = [
            SpeechEnergyWindow {
                range: range(0.8, 0.85),
                rms: 0.4,
            },
            SpeechEnergyWindow {
                range: range(0.85, 0.9),
                rms: 0.2,
            },
        ];
        let out = snap(
            &[vec![piece(0.81, 1.6, ClipRole::Payoff, 0)]],
            &tr,
            &energy,
            &s,
        )
        .unwrap();
        assert_eq!(out[0].segments[0].range.start(), t(expected));
    }
}

#[test]
fn frame_quantization_rounds_subframe_word_gaps_and_refuses_missing_gaps() {
    let mut tr = transcript(true);
    tr.segments[0].words[0].range = Some(range(0.2, 0.811));
    tr.segments[0].words[1].range = Some(range(0.819, 1.6));
    let mut s = settings();
    s.tolerance = t(0.01);
    let candidates = vec![vec![piece(0.815, 1.6, ClipRole::Payoff, 0)]];
    let result = snap(&candidates, &tr, &quiet(), &s).unwrap();
    assert_eq!(result[0].segments[0].range.start(), t(24.0 / 30.0));
    assert_eq!(result[0].adjustments[0].start.kind, BoundaryKind::WordFade);
    // Timing-only maintainer-cut reproduction (synthetic text), beside a
    // continuous 90-second shape. No transcript or private media is committed.
    let maintainer = [
        (0., 4.88),
        (4.88, 9.54),
        (9.54, 14.36),
        (14.36, 19.96),
        (19.96, 26.4),
        (27.12, 30.54),
        (30.54, 34.3),
        (34.3, 36.82),
        (36.82, 42.68),
        (42.68, 43.84),
        (43.84, 46.36),
        (46.36, 51.02),
        (51.02, 56.08),
        (56.82, 60.14),
        (60.14, 62.54),
        (62.54, 67.02),
        (67.02, 72.04),
        (72.04, 75.7),
        (75.7, 79.86),
        (79.86, 84.62),
        (86.52, 88.86),
    ];
    let synthetic: Vec<_> = (0..18)
        .map(|i| (i as f64 * 4.88, (i + 1) as f64 * 4.88))
        .collect();
    for boundaries in [&maintainer[..], &synthetic[..]] {
        for words in [false, true] {
            let mut continuous = transcript(false);
            continuous.segments = boundaries
                .iter()
                .enumerate()
                .map(|(index, &(start, end))| {
                    let middle = (start + end) / 2.;
                    Segment {
                        id: format!("continuous-{index}").into(),
                        range: range(start, end),
                        text: "one two".into(),
                        words: if words {
                            vec![word("one", start, middle), word("two", middle, end)]
                        } else {
                            vec![]
                        },
                        spans: None,
                    }
                })
                .collect();
            let mut limits = settings();
            limits.source_range = range(0., 90.);
            limits.max_duration = t(90.);
            let end = boundaries[3].1;
            let out = snap(
                &[vec![piece(4.88, end, ClipRole::Payoff, 0)]],
                &continuous,
                &[],
                &limits,
            )
            .unwrap();
            assert_eq!(out[0].adjustments[0].start.kind, BoundaryKind::WordFade);
            assert_eq!(out[0].segments[0].range.start(), t(146. / 30.));
        }
    }
    s.tolerance = Tick::ZERO;
    assert!(
        snap(
            &[vec![piece(0.201, 1.6, ClipRole::Payoff, 0)]],
            &tr,
            &[],
            &s
        )
        .is_err()
    );
}

#[test]
fn refuses_post_snap_overlap_duration_and_hostile_inputs() {
    let tr = transcript(true);
    let mut s = settings();
    let candidates = vec![vec![
        piece(0.3, 1.3, ClipRole::Payoff, 0),
        piece(1.2, 1.5, ClipRole::Context, 1),
    ]];
    s.tolerance = t(1.);
    assert!(matches!(
        snap(&candidates, &tr, &quiet(), &s),
        Err(Error::ClipBoundary {
            reason: BoundaryFailure::SegmentOverlap,
            ..
        })
    ));
    s = settings();
    s.max_duration = t(1.);
    assert!(matches!(
        snap(
            &[vec![piece(0.4, 1.5, ClipRole::Payoff, 0)]],
            &tr,
            &quiet(),
            &s
        ),
        Err(Error::ClipBoundary {
            reason: BoundaryFailure::TotalDuration,
            ..
        })
    ));
    s = settings();
    s.min_segment_duration = t(2.);
    assert!(matches!(
        snap(
            &[vec![piece(0.4, 1.5, ClipRole::Payoff, 0)]],
            &tr,
            &quiet(),
            &s
        ),
        Err(Error::ClipBoundary {
            reason: BoundaryFailure::SegmentDuration,
            ..
        })
    ));
    s = settings();
    s.tolerance = Tick(-1);
    assert!(snap(&[], &tr, &[], &s).is_err());
    s = settings();
    s.silence_rms = f64::NAN;
    assert!(snap(&[], &tr, &[], &s).is_err());
    s = settings();
    assert!(snap(&[vec![]], &tr, &[], &s).is_err());
    assert!(snap(&[vec![piece(0.2, 1.6, ClipRole::Payoff, 1)]], &tr, &[], &s).is_err());
    assert!(
        snap(
            &[vec![
                piece(0.2, 1.6, ClipRole::Payoff, 0),
                piece(2.4, 3.6, ClipRole::Hook, 1)
            ]],
            &tr,
            &[],
            &s
        )
        .is_err()
    );
    assert!(
        snap(
            &[],
            &tr,
            &[SpeechEnergyWindow {
                range: range(0., 1.),
                rms: f64::NAN
            }],
            &s
        )
        .is_err()
    );
}

#[test]
fn real_decoded_fixture_has_known_pauses_and_cancellation() {
    use std::sync::{Arc, atomic::AtomicBool};
    let dir = testkit::fixture::dir("clip-boundaries");
    let Some(path) = testkit::oracle::generate(
        &dir,
        "clip boundary audio fixture",
        "known-pauses.wav",
        &[
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=if(between(t\\,0.2\\,1.6)+between(t\\,2.4\\,3.6)+between(t\\,4.4\\,5.6)\\,0.2*sin(2*PI*440*t)\\,0):s=16000:d=6",
            "-c:a",
            "pcm_f32le",
        ],
    ) else {
        return;
    };
    let asset = media::SourceAsset::import(media::AssetId(1), path).unwrap();
    let energy =
        media::analyze_speech_energy(&asset, range(0., 6.), Arc::new(AtomicBool::new(false)))
            .unwrap();
    assert_eq!(energy.len(), 300);
    let partial = media::analyze_speech_energy(
        &asset,
        range(0.005, 0.055),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(partial.len(), 3);
    assert_eq!(partial[0].range, range(0.005, 0.025));
    assert_eq!(partial[2].range, range(0.045, 0.055));
    assert!(partial.iter().all(|w| w.rms == 0.0));
    assert!(matches!(
        media::analyze_speech_energy(&asset, range(0., 7201.), Arc::new(AtomicBool::new(false))),
        Err(media::MediaError::TooLarge(_))
    ));
    for w in &energy {
        let midpoint = (w.range.start() + w.range.end()).0 / 2;
        let second = Tick(midpoint).seconds();
        let quiet = (1.65..2.35).contains(&second) || (3.65..4.35).contains(&second);
        if quiet {
            assert!(w.rms < 0.001);
        }
        if (0.3..1.5).contains(&second) {
            assert!(w.rms > 0.1);
        }
    }
    let result = snap(
        &[vec![piece(0.4, 1.5, ClipRole::Payoff, 0)]],
        &transcript(true),
        &energy,
        &settings(),
    )
    .unwrap();
    assert_eq!(result[0].segments[0].range, range(0.2, 1.6));
    assert!(matches!(
        media::analyze_speech_energy(&asset, range(0., 6.), Arc::new(AtomicBool::new(true))),
        Err(media::MediaError::Cancelled)
    ));
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config {cases:128,rng_seed:proptest::test_runner::RngSeed::Fixed(20902),..Default::default()})]
    #[test]
    fn rational_rates_bound_speech_overlap_and_movement(start in 0.0f64..0.7, end in 1.1f64..2.1, rate in 0usize..11) {
        let tr=transcript(true); let mut s=settings(); s.frame_rate=FrameRate::COMMON[rate];
        if let Ok(output)=snap(&[vec![piece(start,end,ClipRole::Payoff,0)]],&tr,&quiet(),&s) {
            let registry=LanguageRegistry::standard().unwrap();
            let units=captions::source_speech_units(&tr,registry.profile(&tr.source_lang),&LayoutHooks::default()).unwrap();
            for a in [&output[0].adjustments[0].start,&output[0].adjustments[0].end] {
                let allowance = if a.kind == BoundaryKind::WordFade { Tick(s.frame_rate.tick_of(1).unwrap().0/2) } else { Tick::ZERO };
                proptest::prop_assert!(a.delta.abs() <= s.tolerance + allowance);
                proptest::prop_assert_eq!(s.frame_rate.snap(a.adjusted).unwrap(),a.adjusted);
                if a.kind != BoundaryKind::WordFade {
                    proptest::prop_assert!(!units.iter().any(|u| u.range.start()<a.adjusted && a.adjusted<u.range.end()));
                } else {
                    proptest::prop_assert!(units.iter().any(|u| (u.range.end()-a.adjusted).abs() <= Tick(s.frame_rate.tick_of(1).unwrap().0/2) || (u.range.start()-a.adjusted).abs() <= Tick(s.frame_rate.tick_of(1).unwrap().0/2)));
                }
            }
        }
    }
}

#[test]
fn labelled_multilingual_pause_baseline_reduces_mid_speech_cuts_to_zero() {
    // Fixed set and baseline: two injected mid-speech edges per language. The
    // nearest-frame baseline cuts all eight; captions + energy must cut none.
    let mut baseline = 0;
    let mut adjusted = 0;
    for (language, text) in [
        ("en", "Hello."),
        ("vi", "Xin chào."),
        ("zh-Hans", "你好。"),
        ("zh-Hant", "你好。"),
    ] {
        let mut tr = transcript(false);
        tr.source_lang = LanguageTag::parse(language).unwrap();
        tr.segments[0].text = text.into();
        let output = snap(
            &[vec![piece(0.4, 1.5, ClipRole::Payoff, 0)]],
            &tr,
            &quiet(),
            &settings(),
        )
        .unwrap();
        for edge in [t(0.4), t(1.5)] {
            let edge = settings().frame_rate.snap_nearest(edge).unwrap();
            baseline += usize::from(
                tr.segments[0].range.start() < edge && edge < tr.segments[0].range.end(),
            );
        }
        for edge in [
            output[0].segments[0].range.start(),
            output[0].segments[0].range.end(),
        ] {
            adjusted += usize::from(
                tr.segments[0].range.start() < edge && edge < tr.segments[0].range.end(),
            );
        }
        assert_eq!(output[0].segments[0].range, range(0.2, 1.6));
    }
    assert_eq!(baseline, 8);
    assert_eq!(adjusted, 0);
}

#[test]
fn silence_is_nearest_inside_sentence_gap_and_ties_choose_earlier() {
    let tr = transcript(false);
    let candidates = vec![vec![piece(0.2, 2.0, ClipRole::Payoff, 0)]];
    let energy = [
        SpeechEnergyWindow {
            range: range(1.6, 1.8),
            rms: 0.0,
        },
        SpeechEnergyWindow {
            range: range(2.2, 2.4),
            rms: 0.0,
        },
    ];
    let output = snap(&candidates, &tr, &energy, &settings()).unwrap();
    assert_eq!(output[0].segments[0].range.end(), t(1.8));
    assert_eq!(
        output[0].adjustments[0].end.kind,
        BoundaryKind::SentenceSilence
    );
    let output = snap(
        &candidates,
        &tr,
        &[SpeechEnergyWindow {
            range: range(1.7, 1.9),
            rms: 0.0,
        }],
        &settings(),
    )
    .unwrap();
    assert_eq!(output[0].segments[0].range.end(), t(1.9));
}
