#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "validated test fixtures"
)]

use captions::{
    Cue, CueOutcome, CueText, CueTrack, CueWordTiming, FindOptions, LanguageTag, NfcText,
    find_plan, find_replace,
};
use std::collections::{BTreeMap, BTreeSet};
use time::{SourceRange, TICKS_PER_SECOND, Tick};

fn lang(tag: &str) -> LanguageTag {
    LanguageTag::parse(tag).unwrap()
}

fn cue(id: &str, source: &str, text: &str, edited: bool) -> Cue {
    let start = Tick(TICKS_PER_SECOND);
    Cue {
        word_timing: CueWordTiming::Unverified,
        id: id.into(),
        range: SourceRange::new(start, Tick(2 * TICKS_PER_SECOND)).unwrap(),
        source_lang: lang(source),
        texts: BTreeMap::from([(
            lang(source),
            CueText {
                text: text.into(),
                edited,
                translation_ref: None,
            },
        )]),
        style: None,
    }
}

fn track(cues: Vec<Cue>) -> CueTrack {
    CueTrack {
        styles: Vec::new(),
        cues,
    }
}

fn source(track: &CueTrack, id: &str) -> String {
    track
        .cues
        .iter()
        .find(|cue| cue.id.as_str() == id)
        .and_then(|cue| cue.texts.get(&cue.source_lang))
        .map(|text| text.text.as_str().to_owned())
        .unwrap()
}

fn outcome<'a>(plan: &'a captions::FindPlan, id: &str) -> &'a CueOutcome {
    &plan
        .cues
        .iter()
        .find(|cue| cue.cue.as_str() == id)
        .unwrap()
        .outcome
}

#[test]
fn vietnamese_nfd_query_matches_nfc_text_and_round_trips_the_result() {
    // The stored text is NFC; a decomposed query and replacement must meet it in the same form.
    let mut t = track(vec![cue("a", "vi", "Cha\u{0300}o ba\u{0323}n", false)]);
    let plan = find_replace(
        &mut t,
        None,
        "Cha\u{0300}o",
        "Xin chào",
        FindOptions::default(),
    )
    .unwrap();
    assert_eq!(plan.matches, 1);
    assert_eq!(source(&t, "a"), "Xin chào bạn");
    assert!(unicode_normalization::is_nfc(&source(&t, "a")));
}

#[test]
fn chinese_text_matches_by_character_and_keeps_its_characters() {
    let mut t = track(vec![cue("z", "zh-Hans", "你好朋友", false)]);
    let plan = find_plan(&t, None, "朋", "伴", FindOptions::default()).unwrap();
    assert_eq!(plan.matches, 1);
    find_replace(&mut t, None, "朋", "伴", FindOptions::default()).unwrap();
    assert_eq!(source(&t, "z"), "你好伴友");
}

#[test]
fn case_folding_is_optional_and_applies_per_character() {
    let t = track(vec![cue("a", "en", "Hello friend, HELLO again", false)]);
    let folded = find_plan(&t, None, "hello", "Hi", FindOptions::default()).unwrap();
    assert_eq!(folded.matches, 2);
    let exact = find_plan(
        &t,
        None,
        "hello",
        "Hi",
        FindOptions {
            case_sensitive: true,
            whole_word: false,
        },
    )
    .unwrap();
    assert_eq!(exact.matches, 0);
    assert!(exact.cues.is_empty());
}

#[test]
fn vietnamese_uppercase_matches_lowercase_when_folded() {
    let t = track(vec![cue("a", "vi", "Chào bạn", false)]);
    let plan = find_plan(&t, None, "CHÀO", "Xin chào", FindOptions::default()).unwrap();
    assert_eq!(plan.matches, 1);
}

#[test]
fn whole_word_rejects_matches_inside_longer_words() {
    let t = track(vec![cue("a", "en", "cat category cat. cats", false)]);
    let whole = FindOptions {
        case_sensitive: false,
        whole_word: true,
    };
    let plan = find_plan(&t, None, "cat", "dog", whole).unwrap();
    assert_eq!(
        plan.matches, 2,
        "the word before a full stop counts; category and cats do not"
    );
    let any = find_plan(&t, None, "cat", "dog", FindOptions::default()).unwrap();
    assert_eq!(any.matches, 4);
}

#[test]
fn a_match_never_splits_a_grapheme_cluster() {
    // `a` followed by a combining double inverted breve has no precomposed form, so NFC keeps the
    // cluster. The query `a` must not match inside it, or the replacement would strand the mark.
    let t = track(vec![cue("a", "en", "a\u{0361}b a", false)]);
    let plan = find_plan(&t, None, "a", "x", FindOptions::default()).unwrap();
    assert_eq!(plan.matches, 1);
}

#[test]
fn edited_cues_are_protected_reported_and_left_unchanged() {
    let mut t = track(vec![
        cue("manual", "en", "Hello manual", true),
        cue("auto", "en", "Hello auto", false),
    ]);
    let plan = find_replace(&mut t, None, "Hello", "Bye", FindOptions::default()).unwrap();
    assert_eq!(plan.matches, 2);
    assert_eq!(*outcome(&plan, "manual"), CueOutcome::Protected);
    assert_eq!(plan.replaced_matches(), 1);
    assert_eq!(plan.cues_with(|o| matches!(o, CueOutcome::Protected)), 1);
    assert_eq!(source(&t, "manual"), "Hello manual");
    assert_eq!(source(&t, "auto"), "Bye auto");
}

#[test]
fn emptied_and_unchanged_results_are_reported_and_not_written() {
    let mut t = track(vec![
        cue("gone", "en", "Um", false),
        cue("same", "en", "keep", false),
    ]);
    let plan = find_replace(&mut t, None, "Um", "   ", FindOptions::default()).unwrap();
    assert_eq!(*outcome(&plan, "gone"), CueOutcome::Emptied);
    assert_eq!(source(&t, "gone"), "Um");

    let same = find_plan(&t, None, "keep", "keep", FindOptions::default()).unwrap();
    assert_eq!(*outcome(&same, "same"), CueOutcome::Unchanged);
    assert_eq!(same.replaced_matches(), 0);
}

#[test]
fn replacement_never_moves_timings_and_keeps_translations() {
    let mut first = cue("a", "en", "Hello friend", false);
    first.texts.insert(
        lang("vi"),
        CueText {
            text: "Chào bạn".into(),
            edited: false,
            translation_ref: Some("ref".into()),
        },
    );
    let before = first.clone();
    let mut t = track(vec![first]);
    find_replace(&mut t, None, "Hello", "Hi", FindOptions::default()).unwrap();
    let after = t.cues.first().unwrap();
    assert_eq!(after.range, before.range);
    assert_eq!(after.word_timing, before.word_timing);
    assert_eq!(after.texts.get(&lang("vi")), before.texts.get(&lang("vi")));
    assert_eq!(source(&t, "a"), "Hi friend");
    assert!(after.texts.get(&lang("en")).unwrap().edited);
}

#[test]
fn scope_limits_matches_to_the_selected_cues() {
    let mut t = track(vec![
        cue("one", "en", "Hello", false),
        cue("two", "en", "Hello", false),
    ]);
    let scope = BTreeSet::from([NfcText::from("two")]);
    let plan = find_replace(&mut t, Some(&scope), "Hello", "Bye", FindOptions::default()).unwrap();
    assert_eq!(plan.cues.len(), 1);
    assert_eq!(source(&t, "one"), "Hello");
    assert_eq!(source(&t, "two"), "Bye");
}

#[test]
fn the_previewed_plan_is_the_plan_that_is_applied() {
    let t = track(vec![
        cue("a", "en", "Hello one", false),
        cue("b", "en", "Hello two Hello", true),
        cue("c", "en", "Hello three", false),
    ]);
    let preview = find_plan(&t, None, "Hello", "Hey", FindOptions::default()).unwrap();
    let mut applied = t.clone();
    let written = find_replace(&mut applied, None, "Hello", "Hey", FindOptions::default()).unwrap();
    assert_eq!(preview, written);
    assert_eq!(preview.matches, 4);
    assert_eq!(preview.replaced_matches(), 2);
}

#[test]
fn empty_find_text_and_oversized_fields_are_refused() {
    let t = track(vec![cue("a", "en", "Hello", false)]);
    assert!(find_plan(&t, None, "", "x", FindOptions::default()).is_err());
    let long = "x".repeat(513);
    assert!(find_plan(&t, None, "Hello", &long, FindOptions::default()).is_err());
    assert!(find_plan(&t, None, &long, "x", FindOptions::default()).is_err());
}

#[test]
fn the_edited_track_survives_a_json_round_trip() {
    let mut t = track(vec![cue("a", "vi", "Chào bạn", false)]);
    find_replace(&mut t, None, "bạn", "bè", FindOptions::default()).unwrap();
    let json = serde_json::to_string(&t).unwrap();
    let back: CueTrack = serde_json::from_str(&json).unwrap();
    assert_eq!(back.cues, t.cues);
    assert_eq!(source(&back, "a"), "Chào bè");
}
