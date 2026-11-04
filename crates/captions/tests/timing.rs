use captions::{
    DisplayReplacement, DisplayVariant, Fingerprint, Format, LanguageRegistry, LanguageTag,
    LayoutHooks, LineLimit, NormalizedText, Segment, SegmentRules, SpanTiming, TimingKind,
    TimingProvenance, TimingSpans, TimingUnit, Transcript, WarningKind,
};
use proptest::prelude::*;
use time::{FrameRate, SourceRange, TICKS_PER_SECOND, Tick};
use unicode_segmentation::UnicodeSegmentation;

#[allow(clippy::unwrap_used, reason = "validated timing fixture")]
fn language(tag: &str) -> LanguageTag {
    LanguageTag::parse(tag).unwrap()
}
#[allow(clippy::unwrap_used, reason = "validated timing fixture")]
fn range(start: usize, end: usize) -> SourceRange {
    SourceRange::new(
        Tick(start as i64 * TICKS_PER_SECOND),
        Tick(end as i64 * TICKS_PER_SECOND),
    )
    .unwrap()
}
fn input(index: usize, raw: std::ops::Range<usize>, unit: TimingUnit) -> SpanTiming {
    SpanTiming {
        index,
        unit,
        raw,
        range: Some(range(index, index + 1)),
        provenance: TimingProvenance::Native,
        speaker: None,
        confidence: None,
    }
}
fn transcript(raw: &str, spans: TimingSpans, tag: &str, duration: usize) -> Transcript {
    Transcript {
        source_lang: language(tag),
        fingerprint: Fingerprint {
            provider: "test".into(),
            model: "test".into(),
            settings: "test".into(),
        },
        segments: vec![Segment {
            id: "segment".into(),
            text: raw.into(),
            range: range(0, duration),
            words: vec![],
            spans: Some(spans),
        }],
    }
}
#[allow(clippy::unwrap_used, reason = "validated timing fixture")]
fn profile(tag: &str) -> captions::LanguageProfile {
    LanguageRegistry::standard()
        .unwrap()
        .profile(&language(tag))
        .clone()
}
#[allow(clippy::unwrap_used, reason = "validated timing fixture")]
fn rules(tag: &str, width: u32) -> SegmentRules {
    let mut rules = SegmentRules::from_profile(&profile(tag), FrameRate::FPS_24).unwrap();
    rules.min_duration = Tick(1);
    rules.min_gap = Tick::ZERO;
    rules.line_limit = LineLimit::Graphemes(width);
    rules.max_lines = 1;
    rules
}
#[test]
fn split_utf8_tokens_retain_identity_raw_bytes_and_many_to_many_grapheme_spans() {
    let raw = "e\u{302}\u{301}👩🏽‍💻中";
    let inputs: Vec<_> = (0..raw.len())
        .map(|i| input(i, i..i + 1, TimingUnit::Token))
        .collect();
    let spans = TimingSpans::new(raw, inputs.clone(), None).unwrap();
    assert_eq!(spans.normalized().text().as_str(), "ế👩🏽‍💻中");
    assert_eq!(spans.raw(), raw);
    for (i, s) in spans.spans().iter().enumerate() {
        assert_eq!(s.input(), &inputs[i]);
        assert_eq!(s.input().index, i);
        let first = spans.spans().first().unwrap();
        if i < 5 {
            assert_eq!(s.normalized(), first.normalized());
        }
    }
    let restored: TimingSpans =
        serde_json::from_str(&serde_json::to_string(&spans).unwrap()).unwrap();
    assert_eq!(spans, restored);
    let groups = spans.groups(&profile("zh-Hans")).unwrap();
    assert_eq!(groups.len(), 3);
    assert_eq!(groups[0].range, Some(range(0, 5)));
    assert_eq!(groups[0].indices, (0..5).collect::<Vec<_>>());
    assert!(!format!("{spans:?}").contains('ế'));
    assert!(!format!("{spans:?}").contains(raw));
}
#[test]
fn mixed_scripts_group_by_profile_without_splitting_provider_spans() {
    let raw = "中文LatinＡＢ👨‍👩‍👧‍👦，。";
    let inputs = raw
        .grapheme_indices(true)
        .enumerate()
        .map(|(i, (offset, g))| input(i, offset..offset + g.len(), TimingUnit::Character))
        .collect();
    let spans = TimingSpans::new(raw, inputs, None).unwrap();
    let groups = spans.groups(&profile("zh-Hant")).unwrap();
    let values: Vec<_> = groups.iter().map(|g| &raw[g.display.clone()]).collect();
    assert_eq!(values, ["中", "文", "LatinＡＢ", "👨‍👩‍👧‍👦", "，", "。"]);
    assert_eq!(groups[2].range, Some(range(2, 9)));
    assert_eq!(profile("vi").timing_unit, TimingUnit::Syllable);
    assert_eq!(profile("en").timing_unit, TimingUnit::Word);
    assert_eq!(profile("zh-Hans").timing_unit, TimingUnit::Character);
    let latin = TimingSpans::new(
        "Latin",
        vec![
            input(0, 0..2, TimingUnit::Token),
            input(1, 2..5, TimingUnit::Token),
        ],
        None,
    )
    .unwrap();
    assert_eq!(latin.groups(&profile("en")).unwrap().len(), 1);
    let mut tokens = profile("en");
    tokens.timing_unit = TimingUnit::Token;
    let groups = latin.groups(&tokens).unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].display, 0..2);
    // A token crossing Han characters cannot be split into invented timestamps.
    let whole = TimingSpans::new("中文", vec![input(0, 0..6, TimingUnit::Token)], None).unwrap();
    assert_eq!(whole.groups(&profile("zh-Hans")).unwrap().len(), 1);
}
#[test]
fn han_variant_selection_retains_source_timing_through_segmentation_and_srt() {
    let raw = "后发ABC👩🏽‍💻，ＡＢ。";
    let inputs: Vec<_> = raw
        .grapheme_indices(true)
        .enumerate()
        .map(|(i, (offset, g))| input(i, offset..offset + g.len(), TimingUnit::Character))
        .collect();
    let source = TimingSpans::new(raw, inputs.clone(), None).unwrap();
    let display = source
        .with_display(DisplayVariant {
            language: language("zh-Hant"),
            replacements: vec![
                DisplayReplacement {
                    normalized: 0..3,
                    text: "後面".into(),
                },
                DisplayReplacement {
                    normalized: 3..6,
                    text: "發".into(),
                },
            ],
        })
        .unwrap();
    assert_eq!(display.display().as_str(), "後面發ABC👩🏽‍💻，ＡＢ。");
    assert_eq!(display.spans()[0].display(), 0..6);
    assert_eq!(display.spans()[1].display(), 6..9);
    assert_eq!(display.spans()[1].input().range, inputs[1].range);
    let input = transcript(raw, display, "zh-Hans", inputs.len());
    let output = captions::segment_transcript(
        &input,
        &profile("zh-Hant"),
        &rules("zh-Hant", 3),
        &LayoutHooks::default(),
    )
    .unwrap();
    assert!(
        output
            .cues
            .iter()
            .all(|c| c.timing == TimingKind::SourceSpans)
    );
    assert!(
        output
            .cues
            .iter()
            .all(|c| c.cue.source_lang == language("zh-Hans"))
    );
    let track = captions::CueTrack {
        cues: output.cues.iter().map(|c| c.cue.clone()).collect(),
    };
    let compact = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let joined: String = track
        .cues
        .iter()
        .map(|c| c.texts[&language("zh-Hant")].text.as_str())
        .collect();
    assert_eq!(compact(&joined), "後面發ABC👩🏽‍💻，ＡＢ。");
    assert_eq!(track.cues[0].range.start(), Tick::ZERO);
    assert_eq!(track.cues[0].range.end(), range(0, 2).end());
    let srt = captions::export(&track, Format::Srt, &[language("zh-Hant")]).unwrap();
    let restored = captions::parse(srt.text.as_bytes(), Format::Srt, language("zh-Hant")).unwrap();
    assert_eq!(
        restored
            .track
            .cues
            .iter()
            .map(|c| c.range)
            .collect::<Vec<_>>(),
        track.cues.iter().map(|c| c.range).collect::<Vec<_>>()
    );
    let serialized = serde_json::to_string(&input).unwrap();
    assert_eq!(
        serde_json::from_str::<Transcript>(&serialized).unwrap(),
        input
    );
}
#[test]
fn missing_timing_is_retained_and_explicitly_estimated_only_at_segmentation() {
    let mut inputs = vec![
        input(0, 0..3, TimingUnit::Token),
        input(1, 3..6, TimingUnit::Token),
    ];
    inputs[1].range = None;
    let spans = TimingSpans::new("中文", inputs, None).unwrap();
    assert!(spans.spans()[1].input().range.is_none());
    assert!(
        spans.groups(&profile("zh-Hans")).unwrap()[1]
            .range
            .is_none()
    );
    let output = captions::segment_transcript(
        &transcript("中文", spans, "zh-Hans", 2),
        &profile("zh-Hans"),
        &rules("zh-Hans", 1),
        &LayoutHooks::default(),
    )
    .unwrap();
    assert!(
        output
            .cues
            .iter()
            .all(|c| c.timing == TimingKind::Estimated)
    );
    assert!(
        output
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::EstimatedTiming)
    );
}
#[test]
fn malformed_maps_layers_variants_and_segment_bounds_are_rejected() {
    for raw in [
        0..0,
        std::ops::Range { start: 1, end: 0 },
        0..4,
        usize::MAX..usize::MAX,
    ] {
        assert!(TimingSpans::new("中", vec![input(0, raw, TimingUnit::Token)], None).is_err());
    }
    let duplicate = vec![
        input(0, 0..1, TimingUnit::Token),
        input(0, 1..2, TimingUnit::Token),
    ];
    assert!(TimingSpans::new("ab", duplicate, None).is_err());
    let reversed = vec![
        input(0, 1..2, TimingUnit::Token),
        input(1, 0..1, TimingUnit::Token),
    ];
    assert!(TimingSpans::new("ab", reversed, None).is_err());
    let bad_time = vec![
        input(1, 0..1, TimingUnit::Token),
        input(2, 1..2, TimingUnit::Token),
    ];
    let mut bad_time = bad_time;
    bad_time[1].range = Some(range(0, 1));
    assert!(TimingSpans::new("ab", bad_time, None).is_err());
    let source =
        TimingSpans::new("e\u{301}中", vec![input(0, 0..3, TimingUnit::Token)], None).unwrap();
    for replacements in [
        vec![DisplayReplacement {
            normalized: 0..1,
            text: "x".into(),
        }],
        vec![DisplayReplacement {
            normalized: 0..2,
            text: "".into(),
        }],
        vec![
            DisplayReplacement {
                normalized: 0..2,
                text: "x".into(),
            },
            DisplayReplacement {
                normalized: 0..2,
                text: "y".into(),
            },
        ],
    ] {
        assert!(
            source
                .with_display(DisplayVariant {
                    language: language("zh-Hant"),
                    replacements
                })
                .is_err()
        );
    }
    let unrelated = source
        .with_display(DisplayVariant {
            language: language("en"),
            replacements: vec![],
        })
        .unwrap();
    assert!(
        captions::segment_transcript(
            &transcript("e\u{301}中", unrelated, "zh-Hans", 2),
            &profile("en"),
            &rules("en", 10),
            &LayoutHooks::default()
        )
        .is_err()
    );
    let mut wire = serde_json::to_value(&source).unwrap();
    wire["spans"][0]["raw"]["end"] = serde_json::json!(999);
    assert!(serde_json::from_value::<TimingSpans>(wire).is_err());
    let large = "x".repeat(16 * 1024 * 1024 + 1);
    assert!(TimingSpans::new(&large, vec![], None).is_err());
    // Replacement cannot split a grapheme formed across its neighbors.
    let source = TimingSpans::new("ab", vec![], None).unwrap();
    assert!(
        source
            .with_display(DisplayVariant {
                language: language("en"),
                replacements: vec![DisplayReplacement {
                    normalized: 1..2,
                    text: "\u{301}".into()
                }]
            })
            .is_err()
    );
}
proptest! {
    #[test]
    fn arbitrary_byte_token_boundaries_preserve_source_identity_and_group_extent(
        pieces in prop::collection::vec(prop::sample::select(vec!["e\u{302}\u{301}","中","文","Ａ","a"," ","，","👩🏽‍💻"]),1..35),
    ) {
        let raw = pieces.concat();
        let spans = TimingSpans::new(&raw,(0..raw.len()).map(|i|input(i,i..i+1,TimingUnit::Token)).collect(),None).unwrap();
        let map = NormalizedText::new(&raw).unwrap();
        for span in spans.spans() {
            prop_assert_eq!(Some(span.normalized()),map.normalized_range(span.input().raw.clone()));
            prop_assert_eq!(span.input().range,Some(range(span.input().index,span.input().index+1)));
        }
        let groups = spans.groups(&profile("zh-Hans")).unwrap();
        prop_assert_eq!(groups.first().unwrap().display.start,0);
        prop_assert_eq!(groups.last().unwrap().display.end,spans.display().as_str().len());
        let mut seen = Vec::new();
        for group in groups { seen.extend(group.indices); }
        prop_assert_eq!(seen,(0..raw.len()).collect::<Vec<_>>());
    }
}

#[test]
fn sparse_raw_token_coverage_never_fabricates_grapheme_timing() {
    let spans = TimingSpans::new("中", vec![input(0, 0..1, TimingUnit::Token)], None).unwrap();
    assert!(
        spans.groups(&profile("zh-Hans")).unwrap()[0]
            .range
            .is_none()
    );
    let spans =
        TimingSpans::new("e\u{301}", vec![input(0, 0..1, TimingUnit::Token)], None).unwrap();
    assert!(spans.groups(&profile("vi")).unwrap()[0].range.is_none());
}
#[test]
fn source_span_segmentation_reuses_dictionary_boundaries_and_translation_estimates() {
    let raw = "中文词";
    let spans = TimingSpans::new(
        raw,
        (0..3)
            .map(|i| input(i, i * 3..i * 3 + 3, TimingUnit::Character))
            .collect(),
        None,
    )
    .unwrap();
    let input = transcript(raw, spans, "zh-Hans", 3);
    let dictionary = |_text: &str| Ok(std::iter::once(0..6).collect());
    let output = captions::segment_transcript(
        &input,
        &profile("zh-Hans"),
        &rules("zh-Hans", 1),
        &LayoutHooks {
            measure: None,
            words: Some(&dictionary),
        },
    )
    .unwrap();
    assert_eq!(
        output.cues[0].cue.texts[&language("zh-Hans")].text.as_str(),
        "中文"
    );
    assert_eq!(output.cues[0].timing, TimingKind::SourceSpans);
    assert_eq!(output.cues[0].cue.range, range(0, 2));
    let target = captions::CueText {
        text: "Hello world".into(),
        edited: false,
        translation_ref: None,
    };
    let translated = captions::segment_translation(
        &output.cues[0].cue,
        language("en"),
        &target,
        &profile("en"),
        &rules("en", 5),
        &LayoutHooks::default(),
    )
    .unwrap();
    assert!(translated.cues.len() > 1);
    assert!(
        translated
            .cues
            .iter()
            .all(|c| c.timing == TimingKind::Estimated)
    );
}
#[test]
#[ignore = "manual release benchmark, not a CI timing threshold"]
fn timing_span_benchmark() {
    let raw = "Tiếng Việt 中文Latin👩🏽‍💻，ＡＢ。 ".repeat(200);
    let duration = Tick(raw.len() as i64 * time::TICKS_PER_SECOND / 1000);
    let inputs: Vec<_> = (0..raw.len())
        .map(|i| {
            let mut span = input(i, i..i + 1, TimingUnit::Token);
            span.range = Some(
                SourceRange::new(
                    Tick(i as i64 * time::TICKS_PER_SECOND / 1000),
                    Tick((i + 1) as i64 * time::TICKS_PER_SECOND / 1000),
                )
                .unwrap(),
            );
            span
        })
        .collect();
    let profile = profile("zh-Hans");
    let rules = rules("zh-Hans", 16);
    let expected: String = captions::NfcText::from(raw.as_str())
        .as_str()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    for repetition in 0..20 {
        let start = std::time::Instant::now();
        let spans = TimingSpans::new(&raw, inputs.clone(), None).unwrap();
        let mut input = transcript(&raw, spans, "zh-Hans", 1);
        input.segments[0].range = SourceRange::new(Tick::ZERO, duration).unwrap();
        let output =
            captions::segment_transcript(&input, &profile, &rules, &LayoutHooks::default())
                .unwrap();
        let elapsed = start.elapsed();
        let actual: String = output
            .cues
            .iter()
            .flat_map(|c| c.cue.texts[&language("zh-Hans")].text.as_str().chars())
            .filter(|c| !c.is_whitespace())
            .collect();
        assert_eq!(actual, expected);
        // The final input space is formatting, not a spoken/timed glyph.
        let visible_end = Tick((raw.len() - 1) as i64 * time::TICKS_PER_SECOND / 1000);
        assert_eq!(output.cues.last().unwrap().cue.range.end(), visible_end);
        assert!(
            output
                .cues
                .iter()
                .all(|c| c.timing == TimingKind::SourceSpans)
        );
        eprintln!(
            "rep={repetition} raw_bytes={} tokens={} cues={} elapsed_us={} visible_end_drift_ticks=0",
            raw.len(),
            inputs.len(),
            output.cues.len(),
            elapsed.as_micros()
        );
    }
}
