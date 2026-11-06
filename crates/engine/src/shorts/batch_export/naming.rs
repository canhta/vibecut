//! Allocates portable short output names.
use super::{Format, MAX_BASE_BYTES, NameSet, UnicodeSegmentation};
pub(super) fn extension(format: Format) -> &'static str {
    match format {
        Format::Srt => "srt",
        Format::WebVtt => "vtt",
    }
}
impl NameSet {
    pub(super) fn unique(&mut self, base: &str) -> String {
        for attempt in 1..=10_000u32 {
            let suffix = if attempt == 1 {
                String::new()
            } else {
                format!("-{attempt}")
            };
            let allowance = MAX_BASE_BYTES.saturating_sub(suffix.len());
            let mut candidate = fit(base, allowance);
            if candidate.is_empty() {
                candidate.push_str("clip");
            }
            candidate.push_str(&suffix);
            if self.used.insert(candidate.to_lowercase()) {
                return candidate;
            }
        }
        "clip".to_owned()
    }
}

fn fit(value: &str, max_bytes: usize) -> String {
    let mut out = String::new();
    for grapheme in value.graphemes(true) {
        if out.len() + grapheme.len() > max_bytes {
            break;
        }
        out.push_str(grapheme);
    }
    out
}

pub(super) fn name_base(title: &str, index: usize) -> String {
    let mut output = String::new();
    for grapheme in title.graphemes(true) {
        let mut piece = String::new();
        for character in grapheme.chars() {
            if unsafe_filename_character(character) {
                piece.push('-');
            } else {
                piece.push(character);
            }
        }
        if output.len() + piece.len() > MAX_BASE_BYTES {
            break;
        }
        output.push_str(&piece);
    }
    let base = output
        .trim_matches(|character: char| character.is_whitespace() || character == '.')
        .trim()
        .to_owned();
    if base.is_empty() {
        return format!("clip-{:02}", index + 1);
    }
    if reserved_name(&base) {
        return format!("clip-{base}");
    }
    base
}

fn unsafe_filename_character(character: char) -> bool {
    matches!(
        character,
        '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
    ) || character.is_control()
        || matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

fn reserved_name(base: &str) -> bool {
    let stem = base.split('.').next().unwrap_or(base).to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let bytes = stem.as_bytes();
    bytes.len() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && bytes
            .get(3)
            .is_some_and(|digit| matches!(*digit, b'1'..=b'9'))
}
