//! Folds transcript text for matching: compatibility forms, diacritics, case.
use unicode_normalization::UnicodeNormalization;
/// Folds text so that the same words written in different Unicode forms compare equal:
/// NFKC unifies full-width and compatibility forms, NFD exposes combining marks, the
/// marks are dropped, Vietnamese đ/Đ become d/D, and the result is lowercased.
/// Han and other scripts without diacritics pass through unchanged.
pub fn fold(text: &str) -> String {
    let composed: String = text.nfkc().collect();
    composed
        .nfd()
        .filter(|character| !is_combining_mark(*character))
        .map(|character| match character {
            'đ' => 'd',
            'Đ' => 'D',
            other => other,
        })
        .collect::<String>()
        .to_lowercase()
}
fn is_combining_mark(character: char) -> bool {
    matches!(
        u32::from(character),
        0x0300..=0x036F
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x20D0..=0x20FF
            | 0xFE20..=0xFE2F
    )
}
