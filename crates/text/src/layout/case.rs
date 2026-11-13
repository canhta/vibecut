//! Maps source text to the letter case a layout draws.

use std::borrow::Cow;
use unicode_segmentation::UnicodeSegmentation;

/// Letter case applied before shaping. The source string is never changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Case {
    /// The text as given.
    #[default]
    AsTyped,
    Upper,
    Lower,
    /// The first letter of each word upper case; the rest as given.
    Title,
}

/// The source with `case` applied. Each character maps to exactly one character, so
/// character indices into the source stay valid; a character whose case mapping expands
/// (for example `ß` to `SS`) keeps its original form.
pub fn apply_case(source: &str, case: Case) -> Cow<'_, str> {
    match case {
        Case::AsTyped => Cow::Borrowed(source),
        Case::Upper => Cow::Owned(
            source
                .chars()
                .map(|c| single(c.to_uppercase(), c))
                .collect(),
        ),
        Case::Lower => Cow::Owned(
            source
                .chars()
                .map(|c| single(c.to_lowercase(), c))
                .collect(),
        ),
        Case::Title => {
            let mut titled = String::with_capacity(source.len());
            for word in source.split_word_bounds() {
                let mut chars = word.chars();
                match chars.next() {
                    Some(first) if first.is_alphabetic() => {
                        titled.push(single(first.to_uppercase(), first));
                        titled.extend(chars);
                    }
                    _ => titled.push_str(word),
                }
            }
            Cow::Owned(titled)
        }
    }
}

/// The one character a case mapping yields, or the original when it yields several.
fn single(mut mapped: impl Iterator<Item = char>, original: char) -> char {
    match (mapped.next(), mapped.next()) {
        (Some(only), None) => only,
        _ => original,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_case_keeps_the_character_count_and_expanding_letters() {
        let text = "héllo Wörld ß";
        assert_eq!(apply_case(text, Case::AsTyped), text);
        assert_eq!(apply_case(text, Case::Upper), "HÉLLO WÖRLD ß");
        assert_eq!(apply_case(text, Case::Lower), "héllo wörld ß");
        assert_eq!(apply_case(text, Case::Title), "Héllo Wörld ß");
        for case in [Case::Upper, Case::Lower, Case::Title] {
            assert_eq!(apply_case(text, case).chars().count(), text.chars().count());
        }
    }

    #[test]
    fn title_capitalises_each_word_and_keeps_apostrophes_inside_words() {
        assert_eq!(
            apply_case("don't stop, 3rd place", Case::Title),
            "Don't Stop, 3rd Place"
        );
        assert_eq!(apply_case("ALREADY Upper", Case::Title), "ALREADY Upper");
        assert_eq!(apply_case("", Case::Title), "");
    }
}
