#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "validated test fixtures"
)]

use captions::{
    Cue, CueText, CueTrack, Error, FillerCandidate, FillerLexicon, LanguageTag, detect_fillers,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use time::{Rounding, SourceRange, TICKS_PER_SECOND, Tick};

fn lang(tag: &str) -> LanguageTag {
    LanguageTag::parse(tag).unwrap()
}
fn range(start: i64, end: i64) -> SourceRange {
    SourceRange::new(Tick(start), Tick(end)).unwrap()
}
/// Word `i` starts at `1000 + i * 100` and lasts 80 ticks, with a 20-tick gap.
fn words(count: usize) -> Vec<SourceRange> {
    (0..count)
        .map(|i| {
            let start = 1000 + i as i64 * 100;
            range(start, start + 80)
        })
        .collect()
}
fn cue(id: &str, tag: &str, text: &str, words: Vec<SourceRange>, edited: bool) -> Cue {
    let start = words.first().map_or(Tick::ZERO, |range| range.start());
    let end = words
        .last()
        .map_or(Tick(TICKS_PER_SECOND), |range| range.end());
    Cue {
        id: id.into(),
        range: SourceRange::new(start, end).unwrap(),
        source_lang: lang(tag),
        texts: BTreeMap::from([(
            lang(tag),
            CueText {
                text: text.into(),
                edited,
                translation_ref: None,
            },
        )]),
        word_ranges: words,
        style: None,
    }
}
fn detect(track: &CueTrack, tag: &str) -> Vec<FillerCandidate> {
    detect_fillers(track, &FillerLexicon::builtin(&lang(tag)).unwrap()).unwrap()
}
fn span(words: &[SourceRange], start: usize, end: usize) -> SourceRange {
    range(
        words.get(start).map_or(0, |range| range.start().0),
        words.get(end - 1).map_or(0, |range| range.end().0),
    )
}

#[test]
fn shipped_lexicons_match_known_fillers_per_language() {
    struct Case {
        tag: &'static str,
        text: &'static str,
        words: usize,
        expected: &'static [(usize, usize)],
    }
    let cases = [
        Case {
            tag: "en",
            text: "Um, uh, so you know, well I mean that like this.",
            words: 11,
            expected: &[(0, 2), (3, 5), (6, 8), (9, 10)],
        },
        Case {
            tag: "vi",
            text: "Ừm, ờ, cái đó đấy, nhé!",
            words: 6,
            expected: &[(0, 2), (4, 6)],
        },
        Case {
            tag: "zh",
            text: "嗯那个我们这个好",
            words: 8,
            expected: &[(0, 3), (5, 7)],
        },
    ];
    for case in cases {
        let words = words(case.words);
        let track = CueTrack {
            cues: vec![cue("cue", case.tag, case.text, words.clone(), false)],
        };
        let found = detect(&track, case.tag);
        let ranges: Vec<SourceRange> = found.iter().map(|candidate| candidate.range).collect();
        let expected: Vec<SourceRange> = case
            .expected
            .iter()
            .map(|(start, end)| span(&words, *start, *end))
            .collect();
        assert_eq!(ranges, expected, "{} candidates", case.tag);
        assert!(
            found
                .iter()
                .all(|candidate| candidate.cue_id.as_str() == "cue"),
            "{} cue id",
            case.tag
        );
    }
}

#[test]
fn edited_state_and_stable_ids_come_from_the_caption_track() {
    let words = words(2);
    let track = CueTrack {
        cues: vec![cue("cue-7", "en", "Like uh.", words.clone(), true)],
    };
    let first = detect(&track, "en");
    let second = detect(&track, "en");
    assert_eq!(first, second, "repeat runs are identical");
    assert_eq!(first.len(), 1, "adjacent fillers merge");
    let candidate = first.first().expect("one candidate");
    assert!(candidate.edited_cue);
    assert_eq!(candidate.cue_id.as_str(), "cue-7");
    assert_eq!(
        candidate.id.as_str(),
        format!("filler:cue-7:{}", words.first().map_or(0, |r| r.start().0))
    );
}

#[test]
fn only_timed_whole_words_qualify() {
    let filler_words = words(2);
    let timed = cue("timed", "en", "Um uh.", filler_words, false);
    let untimed = cue("untimed", "en", "Um uh.", Vec::new(), false);
    let mismatched = cue("mismatched", "en", "Um uh.", words(1), false);
    let inside_words = cue("inside", "en", "likely umbrella.", words(2), false);
    let track = CueTrack {
        cues: vec![untimed, mismatched, inside_words, timed],
    };
    let found = detect(&track, "en");
    assert_eq!(found.len(), 1, "only the fully timed cue contributes");
    assert_eq!(found.first().expect("candidate").cue_id.as_str(), "timed");
    assert_eq!(
        found.first().expect("candidate").range,
        span(&words(2), 0, 2)
    );
}

#[test]
fn unknown_language_is_a_typed_refusal() {
    let error = FillerLexicon::builtin(&lang("fr")).expect_err("no builtin French lexicon");
    assert!(matches!(error, Error::NoFillerLexicon));
    let regional = FillerLexicon::builtin(&lang("vi-VN")).expect("primary subtag selects vi");
    let track = CueTrack {
        cues: vec![cue("cue", "vi-VN", "Ừm.", words(1), false)],
    };
    assert_eq!(detect_fillers(&track, &regional).expect("detect").len(), 1);
}

#[test]
fn malformed_lexicons_are_typed_errors() {
    let cases = [
        ("", "json"),
        (
            r#"{"language":"!!","references":{"a":"x"},"entries":[{"phrase":"um","references":["a"]}]}"#,
            "language",
        ),
        (
            r#"{"language":"en","references":{"a":"x"},"entries":[]}"#,
            "entry count",
        ),
        (
            r#"{"language":"en","references":{"a":"x"},"entries":[{"phrase":"um","references":[]}]}"#,
            "entry reference",
        ),
        (
            r#"{"language":"en","references":{"a":"x"},"entries":[{"phrase":"um","references":["b"]}]}"#,
            "entry reference",
        ),
        (
            r#"{"language":"en","references":{"a":" "},"entries":[{"phrase":"um","references":["a"]}]}"#,
            "reference",
        ),
        (
            r#"{"language":"en","references":{"a":"x"},"entries":[{"phrase":"!!!","references":["a"]}]}"#,
            "phrase",
        ),
        (
            r#"{"language":"en","references":{"a":"x"},"entries":[{"phrase":"a b c d e f g h i","references":["a"]}]}"#,
            "phrase",
        ),
        (
            r#"{"language":"en","references":{"a":"x"},"entries":[{"phrase":"um","references":["a"]},{"phrase":"UM","references":["a"]}]}"#,
            "duplicate phrase",
        ),
        (
            r#"{"language":"en","references":{"a":"x"},"entries":[{"phrase":"um","references":["a"],"extra":1}]}"#,
            "json",
        ),
    ];
    for (json, detail) in cases {
        let error = FillerLexicon::parse(json).expect_err("malformed lexicon");
        assert!(
            matches!(error, Error::MalformedLexicon(_)),
            "{detail}: {error}"
        );
    }
    let oversized = "x".repeat(600 * 1024);
    assert!(matches!(
        FillerLexicon::parse(&oversized),
        Err(Error::MalformedLexicon("size"))
    ));
}

#[test]
fn a_caller_lexicon_adds_a_language_without_detector_changes() {
    let french = FillerLexicon::parse(
        r#"{"language":"fr","references":{"x":"test"},"entries":[{"phrase":"euh","references":["x"]}]}"#,
    )
    .expect("caller lexicon");
    let track = CueTrack {
        cues: vec![cue("cue", "fr", "Euh, oui.", words(2), false)],
    };
    let found = detect_fillers(&track, &french).expect("detect");
    assert_eq!(found.len(), 1);
    assert_eq!(
        found.first().expect("candidate").range,
        span(&words(2), 0, 1)
    );
}

#[test]
fn lexicon_phrases_are_nfc_normalized() {
    let lexicon = FillerLexicon::parse(
        "{\"language\":\"fr\",\"references\":{\"x\":\"test\"},\"entries\":[{\"phrase\":\"cafe\u{301}\",\"references\":[\"x\"]}]}",
    )
    .expect("decomposed caller lexicon");
    let track = CueTrack {
        cues: vec![cue("cue", "fr", "Café.", words(1), false)],
    };
    assert_eq!(detect_fillers(&track, &lexicon).expect("detect").len(), 1);
}

#[test]
fn longest_phrase_wins_without_overlap() {
    let lexicon = FillerLexicon::parse(
        r#"{"language":"en","references":{"x":"test"},"entries":[{"phrase":"you","references":["x"]},{"phrase":"you know","references":["x"]}]}"#,
    )
    .expect("lexicon");
    let track = CueTrack {
        cues: vec![cue("cue", "en", "You know.", words(2), false)],
    };
    let found = detect_fillers(&track, &lexicon).expect("detect");
    assert_eq!(found.len(), 1);
    assert_eq!(
        found.first().expect("candidate").range,
        span(&words(2), 0, 2)
    );
}

#[test]
fn a_same_primary_tag_finds_the_cue_text() {
    let lexicon = FillerLexicon::builtin(&lang("zh")).expect("zh lexicon");
    let track = CueTrack {
        cues: vec![cue("cue", "zh-Hant", "嗯。", words(1), false)],
    };
    let found = detect_fillers(&track, &lexicon).expect("detect");
    assert_eq!(found.len(), 1);
}

#[test]
fn translated_text_is_never_paired_with_source_word_times() {
    let mut translated = cue("cue", "en", "Um.", words(1), false);
    translated.source_lang = lang("vi");
    let track = CueTrack {
        cues: vec![translated],
    };
    assert!(detect(&track, "en").is_empty());
}

#[test]
fn candidate_ids_are_unique_across_a_track() {
    let track = CueTrack {
        cues: vec![
            cue("a", "en", "Um, uh.", words(2), false),
            cue("b", "en", "Like.", words(1), false),
            cue("c", "en", "Um.", words(1), true),
        ],
    };
    let found = detect(&track, "en");
    let mut ids: Vec<&str> = found
        .iter()
        .map(|candidate| candidate.id.as_str())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), found.len());
}

#[derive(Deserialize)]
struct Asr {
    text: String,
    words: Vec<AsrWord>,
}
#[derive(Deserialize)]
struct AsrWord {
    start: f64,
    end: f64,
    #[serde(rename = "type")]
    kind: String,
}

#[test]
#[ignore = "manual: set VIBECUT_INTERVIEW_ASR to .samples/cuts/09/asr.json"]
fn interview_excerpt_matches_the_manual_filler_read() {
    let Ok(path) = std::env::var("VIBECUT_INTERVIEW_ASR") else {
        return;
    };
    let json = std::fs::read_to_string(path).unwrap();
    let asr: Asr = serde_json::from_str(&json).unwrap();
    let words: Vec<SourceRange> = asr
        .words
        .iter()
        .filter(|word| word.kind == "word")
        .map(|word| {
            SourceRange::new(
                Tick::from_seconds_f64(word.start, Rounding::Nearest).unwrap(),
                Tick::from_seconds_f64(word.end, Rounding::Nearest).unwrap(),
            )
            .unwrap()
        })
        .collect();
    let track = CueTrack {
        cues: vec![Cue {
            id: "interview-09".into(),
            range: SourceRange::new(words.first().unwrap().start(), words.last().unwrap().end())
                .unwrap(),
            source_lang: lang("vi"),
            texts: BTreeMap::from([(
                lang("vi"),
                CueText {
                    text: asr.text.into(),
                    edited: false,
                    translation_ref: None,
                },
            )]),
            word_ranges: words,
            style: None,
        }],
    };
    let found = detect(&track, "vi");
    println!("{:?}", found.iter().map(|c| c.range).collect::<Vec<_>>());
    assert_eq!(found.len(), 1, "the 60 s excerpt has one final particle");
    let candidate = found.first().unwrap();
    assert_eq!(candidate.cue_id.as_str(), "interview-09");
    assert_eq!(
        candidate.range,
        SourceRange::new(
            Tick::from_seconds_f64(33.46, Rounding::Nearest).unwrap(),
            Tick::from_seconds_f64(33.62, Rounding::Nearest).unwrap(),
        )
        .unwrap()
    );
}
