use captions::{
    Cue, CueText, Fingerprint, Format, LanguageProfile, LanguageRegistry, LanguageTag, LineLimit,
    NfcText, NormalizedText, Segment, SegmentRules, Segmentation, TimingKind, Transcript,
    WarningKind, Word, export, segment_transcript, segment_translation,
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
            spans: None,
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
        let output =
            segment_transcript(&transcript(text, tag), &profile(tag), &rules(tag, width)).unwrap();
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
            styles: Vec::new(),
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
            provenance: Some(captions::TimingProvenance::Native),
            text: word.into(),
            range: Some(range(
                i as i64 * TICKS_PER_SECOND,
                (i as i64 + 1) * TICKS_PER_SECOND,
            )),
            speaker: None,
            confidence: None,
        })
        .collect();
    let output = segment_transcript(&input, &profile("en"), &rules("en", 15)).unwrap();
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
    let output = segment_transcript(&input, &profile("en"), &rules("en", 15)).unwrap();
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
    let output = segment_transcript(&input, &profile("en"), &policy).unwrap();
    assert_eq!(output.cues.len(), 2);
    assert_eq!(
        output.cues.first().unwrap().cue.range.end(),
        Tick(TICKS_PER_SECOND)
    );
    for cue in &output.cues {
        assert!(!cue.cue.word_ranges().is_empty());
        assert!(
            cue.cue
                .word_ranges()
                .iter()
                .all(|word| word.start() >= cue.cue.range.start()
                    && word.end() <= cue.cue.range.end())
        );
    }
}

#[test]
fn translations_are_proportional_estimates_and_keep_flags() {
    let source = Cue {
        word_timing: captions::CueWordTiming::Unverified,
        id: "src".into(),
        range: range(100, 160),
        source_lang: lang("en"),
        texts: BTreeMap::new(),
        style: None,
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
fn long_words_and_long_sentences_warn_without_loss() {
    let text = format!(
        "{} {}",
        "antidisestablishmentarianism".repeat(100),
        "Tiếng Việt đẹp. ".repeat(500)
    );
    let output =
        segment_transcript(&transcript(&text, "vi"), &profile("vi"), &rules("vi", 42)).unwrap();
    assert_preserved(&text, "vi", &output);
    assert!(has(&output, WarningKind::TextLayout));
    assert!(has(&output, WarningKind::ReadingSpeed));
    assert!(output.cues.len() > 100);
}

#[test]
fn invalid_inputs_return_errors_and_fallback_preserves_segment() {
    let base = transcript("a b", "en");
    let mut policy = rules("en", 4);
    policy.line_limit = LineLimit::Graphemes(0);
    assert!(segment_transcript(&base, &profile("en"), &policy).is_err());
    let mut input = base.clone();
    input.segments.first_mut().unwrap().words.push(Word {
        provenance: None,
        text: "wrong".into(),
        range: Some(range(0, 1)),
        confidence: None,
        speaker: None,
    });
    let output = segment_transcript(&input, &profile("en"), &rules("en", 4)).unwrap();
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
    assert!(segment_transcript(&input, &profile("en"), &rules("en", 4)).is_err());
    let mut input = base.clone();
    input.segments.push(input.segments.first().unwrap().clone());
    assert!(segment_transcript(&input, &profile("en"), &rules("en", 4)).is_err());
    let mut tiny = base;
    tiny.segments.first_mut().unwrap().range = range(i64::MAX - 1, i64::MAX);
    assert!(segment_transcript(&tiny, &profile("en"), &rules("en", 1)).is_err());
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
            provenance: None,
            text: "one".into(),
            range: Some(range(0, 10)),
            speaker: None,
            confidence: None,
        },
        Word {
            provenance: None,
            text: "two".into(),
            range: Some(range(10, 20)),
            speaker: None,
            confidence: None,
        },
    ];
    let mut policy = rules("en", 3);
    policy.min_gap = Tick(5);
    policy.min_duration = Tick(30);
    let output = segment_transcript(&input, &profile("en"), &policy).unwrap();
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
        let output = segment_transcript(&input,&profile("en"),&rules("en",width)).unwrap();
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
    for timed in [false, true] {
        if timed {
            let segment = input.segments.first_mut().unwrap();
            let tokens: Vec<_> = segment.text.as_str().split_whitespace().collect();
            segment.words = tokens
                .iter()
                .enumerate()
                .map(|(i, token)| Word {
                    provenance: None,
                    text: (*token).into(),
                    range: Some(range(
                        i as i64 * TICKS_PER_SECOND * 600 / tokens.len() as i64,
                        (i as i64 + 1) * TICKS_PER_SECOND * 600 / tokens.len() as i64,
                    )),
                    speaker: None,
                    confidence: None,
                })
                .collect();
        }
        for iteration in 0..20 {
            let begin = std::time::Instant::now();
            let output = segment_transcript(&input, &p, &policy).unwrap();
            let elapsed = begin.elapsed();
            assert_eq!(compact(&contents(&output, "vi")), expected);
            let drift = output.cues.last().unwrap().cue.range.end() - expected_end;
            assert_eq!(drift, Tick::ZERO);
            println!(
                "timed={timed} iteration={iteration} input_bytes={} graphemes={} cues={} elapsed_us={} drift_ticks={}",
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
}

#[test]
fn source_word_silence_extension_and_missing_timing_are_explicit() {
    let mut input = transcript("你好世界", "zh-Hans");
    let segment = input.segments.first_mut().unwrap();
    segment.words = vec![
        Word {
            provenance: None,
            text: "你好".into(),
            range: Some(range(0, 10)),
            speaker: None,
            confidence: None,
        },
        Word {
            provenance: None,
            text: "世界".into(),
            range: Some(range(100, 110)),
            speaker: None,
            confidence: None,
        },
    ];
    let mut policy = rules("zh-Hans", 2);
    policy.min_gap = Tick(5);
    policy.min_duration = Tick(40);
    let output = segment_transcript(&input, &profile("zh-Hans"), &policy).unwrap();
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
    let output = segment_transcript(&input, &profile("zh-Hans"), &rules("zh-Hans", 10)).unwrap();
    assert_eq!(output.cues.first().unwrap().timing, TimingKind::Estimated);
    assert_preserved("你好世界", "zh-Hans", &output);
    let mut policy = rules("zh-Hans", 10);
    policy.max_duration = Tick(100);
    input.segments.first_mut().unwrap().words.clear();
    let output = segment_transcript(&input, &profile("zh-Hans"), &policy).unwrap();
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
        assert!(segment_transcript(&input, &profile("zh-Hans"), &bad).is_err());
    }
    let output = segment_transcript(&input, &profile("zh-Hans"), &base).unwrap();
    assert!(output.cues.iter().any(|c| {
        c.cue
            .texts
            .get(&lang("zh-Hans"))
            .unwrap()
            .text
            .as_str()
            .contains("👨‍👩‍👧‍👦")
    }));
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
            provenance: None,
            text: "👨".into(),
            range: Some(range(0, 1)),
            speaker: None,
            confidence: None,
        },
        Word {
            provenance: None,
            text: "‍👩‍👧‍👦".into(),
            range: Some(range(1, 2)),
            speaker: None,
            confidence: None,
        },
    ];
    let output = segment_transcript(&input, &profile("en"), &rules("en", 1)).unwrap();
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
        segment_transcript(&input, &profile("en"), &rules("en", 1)),
        Err(captions::Error::TooLarge)
    ));
}

#[test]
fn subunit_tick_rounding_only_rejects_empty_final_cues() {
    let mut input = transcript("one two", "en");
    input.segments.first_mut().unwrap().range = range(0, 1);
    let output = segment_transcript(&input, &profile("en"), &rules("en", 20)).unwrap();
    assert_eq!(output.cues.len(), 1);
    assert_eq!(output.cues.first().unwrap().cue.range, range(0, 1));
    assert_eq!(
        output.cues.first().unwrap().timing,
        TimingKind::SourceSegment
    );
    assert!(segment_transcript(&input, &profile("en"), &rules("en", 3)).is_err());
}

#[allow(clippy::unwrap_used, reason = "validated test fixture")]
fn timed_transcript(text: &str, tokens: &[&str]) -> Transcript {
    let mut input = transcript(text, "zh-Hans");
    input.segments.first_mut().unwrap().words = tokens
        .iter()
        .enumerate()
        .map(|(i, token)| Word {
            provenance: None,
            text: (*token).into(),
            range: Some(range(
                i as i64 * TICKS_PER_SECOND,
                (i as i64 + 1) * TICKS_PER_SECOND,
            )),
            speaker: None,
            confidence: None,
        })
        .collect();
    input
}

#[test]
fn timed_boundaries_share_uax_policy_for_lines_and_cues() {
    for max_lines in [1, 2] {
        let mut limits = rules("zh-Hans", 2);
        limits.max_lines = max_lines;
        let output = segment_transcript(
            &timed_transcript("你好，世界", &["你好", "，", "世界"]),
            &profile("zh-Hans"),
            &limits,
        )
        .unwrap();
        assert_preserved("你好，世界", "zh-Hans", &output);
        assert!(
            contents(&output, "zh-Hans")
                .lines()
                .all(|s| !s.trim_start().starts_with('，'))
        );
        assert_eq!(
            output.cues[0].cue.texts[&lang("zh-Hans")].text.as_str(),
            "你好，"
        );
        assert_eq!(output.cues[0].cue.range, range(0, 2 * TICKS_PER_SECOND));
        assert!(has(&output, WarningKind::TextLayout));
    }
}

#[test]
fn native_word_correspondence_survives_segmentation_and_mapping_without_certifying_groups() {
    use captions::{CueWordTiming, EditMapping, TimingProvenance};
    let mut input = transcript("Tie\u{302}\u{301}ng Vie\u{323}\u{302}t", "vi");
    input.segments[0].words = ["Tiếng", "Việt"]
        .iter()
        .enumerate()
        .map(|(i, text)| Word {
            provenance: Some(if i == 0 {
                TimingProvenance::Native
            } else {
                TimingProvenance::Forced
            }),
            text: (*text).into(),
            range: Some(range(
                i as i64 * TICKS_PER_SECOND,
                (i as i64 + 1) * TICKS_PER_SECOND,
            )),
            speaker: None,
            confidence: None,
        })
        .collect();
    let segmented = segment_transcript(&input, &profile("vi"), &rules("vi", 80)).unwrap();
    let cue = &segmented.cues[0].cue;
    let CueWordTiming::Words { text, records, .. } = &cue.word_timing else {
        panic!("real word proof lost");
    };
    assert_eq!(text.as_str(), "Tiếng Việt");
    assert_eq!(text.as_str().get(records[0].bytes.clone()), Some("Tiếng"));
    assert_eq!(text.as_str().get(records[1].bytes.clone()), Some("Việt"));
    let track = captions::CueTrack {
        styles: vec![],
        cues: vec![cue.clone()],
    };
    let mut without = track.clone();
    without.cues[0].word_timing = CueWordTiming::Unverified;
    for format in [Format::Srt, Format::WebVtt] {
        assert_eq!(
            export(&track, format, &[lang("vi")]).unwrap().text,
            export(&without, format, &[lang("vi")]).unwrap().text,
            "word proof never changes subtitle text bytes"
        );
    }
    let mapped = EditMapping::new(vec![(
        range(TICKS_PER_SECOND / 2, TICKS_PER_SECOND * 2),
        Tick(3 * TICKS_PER_SECOND),
    )])
    .unwrap()
    .map_cues(&track)
    .unwrap();
    let CueWordTiming::Words {
        records: shifted, ..
    } = &mapped.cues[0].word_timing
    else {
        panic!("mapped proof lost");
    };
    assert_eq!(
        match shifted[0].projection {
            captions::WordProjection::Visible(range) => range.start(),
            _ => panic!("visible word"),
        },
        Tick(3 * TICKS_PER_SECOND)
    );
    assert_eq!(
        match shifted[1].projection {
            captions::WordProjection::Visible(range) => range.start(),
            _ => panic!("visible word"),
        },
        Tick(7 * TICKS_PER_SECOND / 2)
    );
    let edge = EditMapping::new(vec![(
        range(0, TICKS_PER_SECOND / 2),
        Tick(i64::MAX - TICKS_PER_SECOND / 2),
    )])
    .unwrap()
    .map_cues(&track)
    .unwrap();
    let CueWordTiming::Words {
        records: clipped, ..
    } = &edge.cues[0].word_timing
    else {
        panic!("clipped proof lost");
    };
    assert_eq!(
        match clipped[0].projection {
            captions::WordProjection::Visible(range) => range.end(),
            _ => panic!("visible word"),
        },
        Tick(i64::MAX)
    );
    assert_eq!(
        clipped[1].projection,
        captions::WordProjection::Clipped,
        "outside words remain known, never assigned a fallback time"
    );
    assert_eq!(
        clipped[1].source_range, records[1].source_range,
        "native timing custody survives the empty intersection"
    );
    let long_text = "a".repeat(65536);
    let mut long = transcript(&long_text, "en");
    long.segments[0].words = vec![Word {
        provenance: Some(TimingProvenance::Native),
        text: long_text.into(),
        range: Some(long.segments[0].range),
        speaker: None,
        confidence: None,
    }];
    let generated = segment_transcript(&long, &profile("en"), &rules("en", 80)).unwrap();
    let long_track = captions::CueTrack {
        styles: vec![],
        cues: generated.cues.into_iter().map(|value| value.cue).collect(),
    };
    let many = EditMapping::new(
        (0..300)
            .map(|index| {
                (
                    range(0, 6 * TICKS_PER_SECOND),
                    Tick(index * 6 * TICKS_PER_SECOND),
                )
            })
            .collect(),
    )
    .unwrap();
    assert!(
        matches!(many.map_cues(&long_track), Err(captions::Error::TooLarge)),
        "bound new correspondence amplification before cloning repeated occurrences"
    );
    for word in &mut input.segments[0].words {
        word.provenance = None;
    }
    assert!(
        segment_transcript(&input, &profile("vi"), &rules("vi", 80))
            .unwrap()
            .cues
            .iter()
            .all(|value| value.cue.word_timing == CueWordTiming::Unverified)
    );
    let mut grouped = transcript("don't", "en");
    grouped.segments[0].words = ["don", "'t"]
        .iter()
        .enumerate()
        .map(|(i, text)| Word {
            provenance: Some(TimingProvenance::Native),
            text: (*text).into(),
            range: Some(range(
                i as i64 * TICKS_PER_SECOND,
                (i as i64 + 1) * TICKS_PER_SECOND,
            )),
            speaker: None,
            confidence: None,
        })
        .collect();
    assert!(
        segment_transcript(&grouped, &profile("en"), &rules("en", 80))
            .unwrap()
            .cues
            .iter()
            .all(|value| value.cue.word_timing == CueWordTiming::Unverified)
    );
}
