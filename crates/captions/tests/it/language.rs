//! Verifies the language-owned primary-subtag and word-spacing rules used when
//! cue text is joined.
#![allow(clippy::unwrap_used, reason = "validated test fixture")]
use captions::LanguageTag;

fn tag(value: &str) -> LanguageTag {
    LanguageTag::parse(value).unwrap()
}

#[test]
fn each_language_row_reports_its_primary_subtag_and_word_spacing() {
    for (language, primary, spaced) in [
        ("vi", "vi", true),
        ("vi-VN", "vi", true),
        ("en", "en", true),
        ("en-US", "en", true),
        ("EN-us", "en", true),
        ("zh", "zh", false),
        ("zh-Hans", "zh", false),
        ("zh-Hant", "zh", false),
        ("zh-Hant-TW", "zh", false),
        ("zh-CN", "zh", false),
        ("ja", "ja", false),
        ("ja-JP", "ja", false),
        ("ko", "ko", true),
        ("th", "th", true),
        ("und", "und", true),
    ] {
        let tag = tag(language);
        assert_eq!(tag.primary(), primary, "{language} primary subtag");
        assert_eq!(tag.joins_with_space(), spaced, "{language} word spacing");
    }
}
