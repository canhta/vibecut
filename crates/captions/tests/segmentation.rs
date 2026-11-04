use captions::{
    Cue, CueText, Fingerprint, Format, LanguageProfile, LanguageRegistry, LanguageTag, LayoutHooks,
    LineLimit, NfcText, NormalizedText, Segment, SegmentRules, Segmentation, TimingKind,
    Transcript, WarningKind, Word, export, segment_transcript, segment_translation,
};
use proptest::prelude::*;
use std::collections::BTreeMap;
use time::{FrameRate, SourceRange, TICKS_PER_SECOND, Tick};
use unicode_segmentation::UnicodeSegmentation;

#[allow(clippy::unwrap_used, reason = "validated test fixture")]
fn lang(tag: &str) -> LanguageTag {
    LanguageTag::parse(tag).unwrap()
}
#[allow(clippy::unwrap_used, reason = "validated test fixture")]
fn range(a: i64, b: i64) -> SourceRange {
    SourceRange::new(Tick(a), Tick(b)).unwrap()
}
#[allow(clippy::unwrap_used, reason = "validated test fixture")]
fn profile(tag: &str) -> LanguageProfile {
    LanguageRegistry::standard()
        .unwrap()
        .profile(&lang(tag))
        .clone()
}
#[allow(clippy::unwrap_used, reason = "validated test fixture")]
fn rules(tag: &str, width: u32) -> SegmentRules {
    let mut rules = SegmentRules::from_profile(&profile(tag), FrameRate::FPS_24).unwrap();
    rules.line_limit = LineLimit::Graphemes(width);
    rules.max_lines = 1;
    rules.min_duration = Tick(1);
    rules.min_gap = Tick::ZERO;
    rules
}
fn transcript(text: &str, tag: &str) -> Transcript {
    Transcript {
        source_lang: lang(tag),
        fingerprint: Fingerprint {
            provider: "test".into(),
            model: "test".into(),
            settings: "test".into(),
        },
        segments: vec![Segment {
            id: "segment".into(),
            range: range(0, TICKS_PER_SECOND * 6),
            text: text.into(),
            words: vec![],
        }],
    }
}
#[allow(clippy::unwrap_used, reason = "validated test fixture")]
fn contents(output: &Segmentation, tag: &str) -> String {
    output
        .cues
        .iter()
        .map(|c| c.cue.texts.get(&lang(tag)).unwrap().text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}
fn compact(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}
fn assert_preserved(text: &str, tag: &str, output: &Segmentation) {
    assert_eq!(
        compact(&contents(output, tag)),
        compact(NfcText::from(text).as_str())
    );
}
fn has(output: &Segmentation, kind: WarningKind) -> bool {
    output.warnings.iter().any(|w| w.kind == kind)
}

#[test]
fn unicode_tables_preserve_text_and_export() {
    for (tag, text, width) in [
        ("vi", "Tiếng Việt rất đẹp, chúng tôi yêu tiếng mẹ đẻ!", 14),
        (
            "en",
            "A long sentence, with punctuation and a final thought.",
            15,
        ),
        ("zh-Hans", "你好，世界！我们热爱语言。", 5),
        ("zh-Hant", "你好，世界！我們熱愛語言。", 5),
        ("zh-Hans", "中文与LatinABC混排👨‍👩‍👧‍👦，全角ＡＢＣ。", 12),
    ] {
        let output = segment_transcript(
            &transcript(text, tag),
            &profile(tag),
            &rules(tag, width),
            &LayoutHooks::default(),
        )
        .unwrap();
        assert_preserved(text, tag, &output);
        for cue in &output.cues {
            assert!(cue.cue.range.start() >= Tick::ZERO);
            assert!(cue.cue.range.end() <= Tick(TICKS_PER_SECOND * 6));
            assert!(
                cue.cue
                    .texts
                    .get(&lang(tag))
                    .unwrap()
                    .text
                    .as_str()
                    .graphemes(true)
                    .count()
                    <= width as usize,
                "{tag}: {:?}",
                cue.cue.texts.get(&lang(tag)).unwrap().text.as_str()
            );
        }
        let track = captions::CueTrack {
            cues: output.cues.iter().map(|c| c.cue.clone()).collect(),
        };
        for format in [Format::Srt, Format::WebVtt] {
            let encoded = export(&track, format, &[lang(tag)]).unwrap();
            let decoded = captions::parse(encoded.text.as_bytes(), format, lang(tag)).unwrap();
            assert_eq!(decoded.track.cues.len(), track.cues.len());
            let joined = decoded
                .track
                .cues
                .iter()
                .map(|c| c.texts.get(&lang(tag)).unwrap().text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            assert_eq!(compact(&joined), compact(NfcText::from(text).as_str()));
        }
    }
}

#[test]
fn normalization_maps_whole_graphemes_many_to_one() {
    let raw = "Tiếng 👩🏽‍💻";
    let map = NormalizedText::new(raw).unwrap();
    assert_eq!(map.text().as_str(), "Tiếng 👩🏽‍💻");
    let span = map.spans().get(2).unwrap();
    assert_eq!(raw.get(span.raw.clone()), Some("ế"));
    assert_eq!(map.text().as_str().get(span.normalized.clone()), Some("ế"));
    assert_eq!(
        map.raw_range(span.normalized.start..span.normalized.start + 1),
        Some(span.raw.clone())
    );
    assert_eq!(map.raw_range(0..0), None);
    assert_eq!(map.raw_range(0..usize::MAX), None);
    assert!(!format!("{map:?}").contains("Tiếng"));
}

#[test]
fn timed_words_choose_sentence_then_clause_and_pause() {
    let mut input = transcript("One. two, three four five", "en");
    let segment = input.segments.first_mut().unwrap();
    segment.words = ["One.", "two,", "three", "four", "five"]
        .into_iter()
        .enumerate()
        .map(|(i, word)| Word {
            text: word.into(),
            range: Some(range(
                i as i64 * TICKS_PER_SECOND,
                (i as i64 + 1) * TICKS_PER_SECOND,
            )),
            speaker: None,
            confidence: None,
        })
        .collect();
    let output = segment_transcript(
        &input,
        &profile("en"),
        &rules("en", 15),
        &LayoutHooks::default(),
    )
    .unwrap();
    assert_eq!(
        output
            .cues
            .first()
            .unwrap()
            .cue
            .texts
            .get(&lang("en"))
            .unwrap()
            .text
            .as_str(),
        "One."
    );
    assert!(
        output
            .cues
            .iter()
            .all(|c| c.timing == TimingKind::SourceWords)
    );
    assert_preserved("One. two, three four five", "en", &output);
    let segment = input.segments.first_mut().unwrap();
    segment.words.first_mut().unwrap().text = "One,".into();
    segment.text = "One, two, three four five".into();
    let output = segment_transcript(
        &input,
        &profile("en"),
        &rules("en", 15),
        &LayoutHooks::default(),
    )
    .unwrap();
    assert_eq!(
        output
            .cues
            .first()
            .unwrap()
            .cue
            .texts
            .get(&lang("en"))
            .unwrap()
            .text
            .as_str(),
        "One, two,"
    );
    let segment = input.segments.first_mut().unwrap();
    segment.words.get_mut(1).unwrap().range =
        Some(range(TICKS_PER_SECOND * 2, TICKS_PER_SECOND * 3));
    segment.words.get_mut(2).unwrap().range =
        Some(range(TICKS_PER_SECOND * 3, TICKS_PER_SECOND * 4));
    segment.words.get_mut(3).unwrap().range =
        Some(range(TICKS_PER_SECOND * 4, TICKS_PER_SECOND * 5));
    segment.words.get_mut(4).unwrap().range =
        Some(range(TICKS_PER_SECOND * 5, TICKS_PER_SECOND * 6));
    let mut policy = rules("en", 100);
    policy.max_duration = Tick(TICKS_PER_SECOND * 20);
    let output =
        segment_transcript(&input, &profile("en"), &policy, &LayoutHooks::default()).unwrap();
    assert_eq!(output.cues.len(), 2);
    assert_eq!(
        output.cues.first().unwrap().cue.range.end(),
        Tick(TICKS_PER_SECOND)
    );
}

#[test]
fn translations_are_proportional_estimates_and_keep_flags() {
    let source = Cue {
        id: "src".into(),
        range: range(100, 160),
        source_lang: lang("en"),
        texts: BTreeMap::new(),
    };
    let translated = CueText {
        text: "一二三四五六".into(),
        edited: true,
        translation_ref: Some("translation".into()),
    };
    let output = segment_translation(
        &source,
        lang("zh-Hans"),
        &translated,
        &profile("zh-Hans"),
        &rules("zh-Hans", 2),
        &LayoutHooks::default(),
    )
    .unwrap();
    assert_eq!(output.cues.len(), 3);
    for (index, entry) in output.cues.iter().enumerate() {
        assert_eq!(entry.timing, TimingKind::Estimated);
        assert_eq!(
            entry.cue.range,
            range(100 + index as i64 * 20, 120 + index as i64 * 20)
        );
        let text = entry.cue.texts.get(&lang("zh-Hans")).unwrap();
        assert!(text.edited);
        assert_eq!(text.translation_ref, translated.translation_ref);
        assert_eq!(entry.cue.source_lang, lang("en"));
    }
    assert!(has(&output, WarningKind::EstimatedTiming));
    assert!(has(&output, WarningKind::ReadingSpeed));
    assert_preserved(translated.text.as_str(), "zh-Hans", &output);
}

#[test]
fn optional_measurement_and_dictionary_protect_words() {
    let input = transcript("一二三四五六", "zh-Hans");
    let dictionary = |_: &str| Ok(vec![0..9, 9..18]);
    let hooks = LayoutHooks {
        measure: None,
        words: Some(&dictionary),
    };
    let output =
        segment_transcript(&input, &profile("zh-Hans"), &rules("zh-Hans", 2), &hooks).unwrap();
    assert_eq!(output.cues.len(), 2);
    assert!(has(&output, WarningKind::TextLayout));
    assert_eq!(
        output
            .cues
            .first()
            .unwrap()
            .cue
            .texts
            .get(&lang("zh-Hans"))
            .unwrap()
            .text
            .as_str(),
        "一二三"
    );
    let measure = |text: &str| Ok(text.graphemes(true).count() as f64 * 10.0);
    let hooks = LayoutHooks {
        measure: Some(&measure),
        words: None,
    };
    let mut policy = rules("zh-Hans", 2);
    policy.line_limit = LineLimit::Pixels(20.0);
    let output = segment_transcript(&input, &profile("zh-Hans"), &policy, &hooks).unwrap();
    assert_eq!(output.cues.len(), 3);
    assert!(!has(&output, WarningKind::TextLayout));
}

#[test]
fn long_words_and_long_sentences_warn_without_loss() {
    let text = format!(
        "{} {}",
        "antidisestablishmentarianism".repeat(100),
        "Tiếng Việt đẹp. ".repeat(500)
    );
    let output = segment_transcript(
        &transcript(&text, "vi"),
        &profile("vi"),
        &rules("vi", 42),
        &LayoutHooks::default(),
    )
    .unwrap();
    assert_preserved(&text, "vi", &output);
    assert!(has(&output, WarningKind::TextLayout));
    assert!(has(&output, WarningKind::ReadingSpeed));
    assert!(output.cues.len() > 100);
}

#[test]
fn invalid_inputs_return_errors_and_fallback_preserves_segment() {
    let base = transcript("a b", "en");
    for bad in [
        LineLimit::Graphemes(0),
        LineLimit::Pixels(f64::NAN),
        LineLimit::Pixels(5.0),
    ] {
        let mut policy = rules("en", 4);
        policy.line_limit = bad;
        assert!(
            segment_transcript(&base, &profile("en"), &policy, &LayoutHooks::default()).is_err()
        );
    }
    let bad_measure = |_: &str| Ok(f64::INFINITY);
    let mut policy = rules("en", 4);
    policy.line_limit = LineLimit::Pixels(5.0);
    assert!(
        segment_transcript(
            &base,
            &profile("en"),
            &policy,
            &LayoutHooks {
                measure: Some(&bad_measure),
                words: None
            }
        )
        .is_err()
    );
    let bad_dictionary = |_: &str| Ok(vec![0..1, 0..2]);
    assert!(
        segment_transcript(
            &base,
            &profile("en"),
            &rules("en", 4),
            &LayoutHooks {
                measure: None,
                words: Some(&bad_dictionary)
            }
        )
        .is_err()
    );
    let mut input = base.clone();
    input.segments.first_mut().unwrap().words.push(Word {
        text: "wrong".into(),
        range: Some(range(0, 1)),
        confidence: None,
        speaker: None,
    });
    let output = segment_transcript(
        &input,
        &profile("en"),
        &rules("en", 4),
        &LayoutHooks::default(),
    )
    .unwrap();
    assert_preserved("a b", "en", &output);
    assert_eq!(output.cues.first().unwrap().timing, TimingKind::Estimated);
    input
        .segments
        .first_mut()
        .unwrap()
        .words
        .first_mut()
        .unwrap()
        .range = Some(range(-1, 1));
    assert!(
        segment_transcript(
            &input,
            &profile("en"),
            &rules("en", 4),
            &LayoutHooks::default()
        )
        .is_err()
    );
    let mut input = base.clone();
    input.segments.push(input.segments.first().unwrap().clone());
    assert!(
        segment_transcript(
            &input,
            &profile("en"),
            &rules("en", 4),
            &LayoutHooks::default()
        )
        .is_err()
    );
    let mut tiny = base;
    tiny.segments.first_mut().unwrap().range = range(i64::MAX - 1, i64::MAX);
    assert!(
        segment_transcript(
            &tiny,
            &profile("en"),
            &rules("en", 1),
            &LayoutHooks::default()
        )
        .is_err()
    );
}

#[test]
fn profile_defaults_and_unsatisfied_limits_are_explicit() {
    for (tag, width, cps) in [
        ("vi", 42, 17),
        ("en", 42, 20),
        ("zh-Hans", 16, 9),
        ("zh-Hant", 16, 9),
    ] {
        let p = profile(tag);
        assert_eq!((p.max_graphemes, p.graphemes_per_second), (width, cps));
        let r = SegmentRules::from_profile(&p, FrameRate::FPS_24).unwrap();
        assert_eq!(r.min_gap, Tick(TICKS_PER_SECOND / 12));
    }
    let mut input = transcript("one two", "en");
    let s = input.segments.first_mut().unwrap();
    s.words = vec![
        Word {
            text: "one".into(),
            range: Some(range(0, 10)),
            speaker: None,
            confidence: None,
        },
        Word {
            text: "two".into(),
            range: Some(range(10, 20)),
            speaker: None,
            confidence: None,
        },
    ];
    let mut policy = rules("en", 3);
    policy.min_gap = Tick(5);
    policy.min_duration = Tick(30);
    let output =
        segment_transcript(&input, &profile("en"), &policy, &LayoutHooks::default()).unwrap();
    assert!(has(&output, WarningKind::MinimumGap));
    assert!(has(&output, WarningKind::Duration));
    assert_eq!(output.cues.first().unwrap().cue.range, range(0, 10));
}

proptest! {
    #[test]
    fn untimed_segmentation_preserves_text_and_monotonic_time(
        words in prop::collection::vec("[a-z]{1,8}", 1..60), width in 8u32..40, duration in 1000i64..1000000,
    ) {
        let text = words.join(" ");
        let input = transcript(&text,"en");
        let mut input = input; input.segments.first_mut().unwrap().range = range(0,duration);
        let output = segment_transcript(&input,&profile("en"),&rules("en",width),&LayoutHooks::default()).unwrap();
        prop_assert_eq!(compact(&contents(&output,"en")),compact(&text));
        let mut end=Tick::ZERO;
        for entry in output.cues {
            prop_assert!(entry.cue.range.start()>=end);
            prop_assert!(entry.cue.range.end()<=Tick(duration));
            end=entry.cue.range.end();
        }
    }
}

#[test]
#[ignore = "manual release benchmark, not a CI timing threshold"]
fn segmentation_benchmark() {
    let text = "Tiếng Việt đẹp, chúng ta giữ nguyên dấu! ".repeat(1000);
    let mut input = transcript(&text, "vi");
    let segment = input.segments.first_mut().unwrap();
    segment.range = range(0, TICKS_PER_SECOND * 600);
    let expected_end = segment.range.end();
    let p = profile("vi");
    let policy = SegmentRules::from_profile(&p, FrameRate::FPS_24).unwrap();
    let expected = compact(NfcText::from(text.as_str()).as_str());
    for iteration in 0..20 {
        let begin = std::time::Instant::now();
        let output = segment_transcript(&input, &p, &policy, &LayoutHooks::default()).unwrap();
        let elapsed = begin.elapsed();
        assert_eq!(compact(&contents(&output, "vi")), expected);
        let drift = output.cues.last().unwrap().cue.range.end() - expected_end;
        assert_eq!(drift, Tick::ZERO);
        println!(
            "iteration={iteration} input_bytes={} graphemes={} cues={} elapsed_us={} drift_ticks={}",
            text.len(),
            NfcText::from(text.as_str())
                .as_str()
                .graphemes(true)
                .count(),
            output.cues.len(),
            elapsed.as_micros(),
            drift.0
        );
    }
}

#[test]
fn source_word_silence_extension_and_missing_timing_are_explicit() {
    let mut input = transcript("你好世界", "zh-Hans");
    let segment = input.segments.first_mut().unwrap();
    segment.words = vec![
        Word {
            text: "你好".into(),
            range: Some(range(0, 10)),
            speaker: None,
            confidence: None,
        },
        Word {
            text: "世界".into(),
            range: Some(range(100, 110)),
            speaker: None,
            confidence: None,
        },
    ];
    let mut policy = rules("zh-Hans", 2);
    policy.min_gap = Tick(5);
    policy.min_duration = Tick(40);
    let output = segment_transcript(
        &input,
        &profile("zh-Hans"),
        &policy,
        &LayoutHooks::default(),
    )
    .unwrap();
    assert_eq!(output.cues.first().unwrap().cue.range, range(0, 40));
    assert_eq!(output.cues.last().unwrap().cue.range, range(100, 110));
    assert_preserved("你好世界", "zh-Hans", &output);
    input
        .segments
        .first_mut()
        .unwrap()
        .words
        .first_mut()
        .unwrap()
        .range = None;
    let output = segment_transcript(
        &input,
        &profile("zh-Hans"),
        &rules("zh-Hans", 10),
        &LayoutHooks::default(),
    )
    .unwrap();
    assert_eq!(output.cues.first().unwrap().timing, TimingKind::Estimated);
    assert_preserved("你好世界", "zh-Hans", &output);
    let mut policy = rules("zh-Hans", 10);
    policy.max_duration = Tick(100);
    input.segments.first_mut().unwrap().words.clear();
    let output = segment_transcript(
        &input,
        &profile("zh-Hans"),
        &policy,
        &LayoutHooks::default(),
    )
    .unwrap();
    assert!(output.cues.len() > 1);
    assert!(has(&output, WarningKind::Duration));
}

#[test]
fn malformed_rule_table_and_grapheme_protection() {
    let input = transcript("👨‍👩‍👧‍👦一家人。", "zh-Hans");
    let base = rules("zh-Hans", 2);
    for bad in [
        SegmentRules {
            max_lines: 0,
            ..base.clone()
        },
        SegmentRules {
            max_cps: 0,
            ..base.clone()
        },
        SegmentRules {
            min_duration: Tick::ZERO,
            ..base.clone()
        },
        SegmentRules {
            max_duration: Tick::ZERO,
            ..base.clone()
        },
        SegmentRules {
            min_gap: Tick(-1),
            ..base.clone()
        },
        SegmentRules {
            break_on_pause: Some(Tick::ZERO),
            ..base.clone()
        },
    ] {
        assert!(
            segment_transcript(&input, &profile("zh-Hans"), &bad, &LayoutHooks::default()).is_err()
        );
    }
    let output =
        segment_transcript(&input, &profile("zh-Hans"), &base, &LayoutHooks::default()).unwrap();
    assert!(output.cues.iter().any(|c| {
        c.cue
            .texts
            .get(&lang("zh-Hans"))
            .unwrap()
            .text
            .as_str()
            .contains("👨‍👩‍👧‍👦")
    }));
    let bad_dictionary = |_: &str| Ok(std::iter::once(0..4).collect());
    assert!(
        segment_transcript(
            &input,
            &profile("zh-Hans"),
            &base,
            &LayoutHooks {
                measure: None,
                words: Some(&bad_dictionary)
            }
        )
        .is_err()
    );
    for raw in ["a\u{315}\u{300}", "각", "👨‍👩‍👧‍👦"] {
        let map = NormalizedText::new(raw).unwrap();
        assert_eq!(map.spans().len(), 1);
        assert_eq!(
            map.raw_range(0..map.text().as_str().len()),
            Some(0..raw.len())
        );
    }
}

#[test]
fn provider_words_cannot_split_a_grapheme_and_oversize_input_is_refused() {
    let mut input = transcript("👨‍👩‍👧‍👦", "en");
    input.segments.first_mut().unwrap().words = vec![
        Word {
            text: "👨".into(),
            range: Some(range(0, 1)),
            speaker: None,
            confidence: None,
        },
        Word {
            text: "‍👩‍👧‍👦".into(),
            range: Some(range(1, 2)),
            speaker: None,
            confidence: None,
        },
    ];
    let output = segment_transcript(
        &input,
        &profile("en"),
        &rules("en", 1),
        &LayoutHooks::default(),
    )
    .unwrap();
    assert_eq!(output.cues.len(), 1);
    assert_eq!(output.cues.first().unwrap().timing, TimingKind::Estimated);
    assert_eq!(contents(&output, "en"), "👨‍👩‍👧‍👦");
    let oversized = "a".repeat(16 * 1024 * 1024 + 1);
    assert!(matches!(
        NormalizedText::new(&oversized),
        Err(captions::Error::TooLarge)
    ));
}

#[test]
fn repeated_identity_output_is_bounded() {
    let mut input = transcript(&"a ".repeat(100), "en");
    input.segments.first_mut().unwrap().id = "x".repeat(200_000).into();
    assert!(matches!(
        segment_transcript(
            &input,
            &profile("en"),
            &rules("en", 1),
            &LayoutHooks::default()
        ),
        Err(captions::Error::TooLarge)
    ));
}

#[test]
fn subunit_tick_rounding_only_rejects_empty_final_cues() {
    let mut input = transcript("one two", "en");
    input.segments.first_mut().unwrap().range = range(0, 1);
    let output = segment_transcript(
        &input,
        &profile("en"),
        &rules("en", 20),
        &LayoutHooks::default(),
    )
    .unwrap();
    assert_eq!(output.cues.len(), 1);
    assert_eq!(output.cues.first().unwrap().cue.range, range(0, 1));
    assert_eq!(
        output.cues.first().unwrap().timing,
        TimingKind::SourceSegment
    );
    assert!(
        segment_transcript(
            &input,
            &profile("en"),
            &rules("en", 3),
            &LayoutHooks::default()
        )
        .is_err()
    );
}
