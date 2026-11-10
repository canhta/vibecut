//! Verifies the language-owned word-spacing rule used when cue text is joined.
#![allow(clippy::unwrap_used, reason = "validated test fixture")]
use captions::LanguageTag;

fn tag(value: &str) -> LanguageTag {
    LanguageTag::parse(value).unwrap()
}

#[test]
fn spaced_languages_join_with_a_space_and_han_kana_languages_without_one() {
    for (language, spaced) in [
        ("vi", true),
        ("vi-VN", true),
        ("en", true),
        ("en-US", true),
        ("zh", false),
        ("zh-Hans", false),
        ("zh-Hant", false),
        ("zh-CN", false),
        ("ja", false),
        ("ja-JP", false),
        ("ko", true),
        ("th", true),
    ] {
        assert_eq!(
            tag(language).joins_with_space(),
            spaced,
            "{language} word spacing"
        );
    }
}
