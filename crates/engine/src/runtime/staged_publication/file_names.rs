//! Produces portable output file names.
use captions::NfcText;
use unicode_segmentation::UnicodeSegmentation;

const MAX_FILE_NAME_BYTES: usize = 255;
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Normalize a project name to NFC, replacing forbidden characters with `_`.
/// Empty names use `Untitled Project`; device names receive an underscore prefix.
/// The result uses at most 255 UTF-8 bytes without splitting a grapheme.
pub fn sanitize_file_name(name: &str) -> String {
    sanitize(name, "Untitled Project", '_', MAX_FILE_NAME_BYTES)
}

/// Build a portable export name within 255 UTF-8 bytes, including the extension.
/// The extension comes from the caller's admitted export format.
pub fn default_file_name(project_name: &str, extension: &str) -> String {
    let extension = bounded(extension, MAX_FILE_NAME_BYTES - 1);
    let suffix = format!(".{extension}");
    let base = sanitize(
        project_name,
        "Untitled Project",
        '_',
        MAX_FILE_NAME_BYTES - suffix.len(),
    );
    format!("{base}{suffix}")
}

pub(crate) fn snapshot_project_name(name: &str, maximum: usize) -> String {
    sanitize(name, "snapshot", '-', maximum)
}

fn sanitize(name: &str, fallback: &str, replacement: char, maximum: usize) -> String {
    let normalized = NfcText::from(name);
    let sanitized: String = normalized
        .as_str()
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
            {
                replacement
            } else {
                character
            }
        })
        .collect();
    let trimmed = sanitized.trim().trim_end_matches(['.', ' ']).trim_end();
    let base = bounded(
        if trimmed.is_empty() {
            fallback
        } else {
            trimmed
        },
        maximum,
    )
    .trim_end_matches(['.', ' ']);
    let base = if base.is_empty() {
        bounded(fallback, maximum)
    } else {
        base
    };
    let stem = base.split('.').next().unwrap_or(base).trim_end();
    if RESERVED
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(stem))
    {
        format!("_{}", bounded(base, maximum.saturating_sub(1)))
    } else {
        base.to_owned()
    }
}

fn bounded(value: &str, maximum: usize) -> &str {
    let end = value
        .grapheme_indices(true)
        .take_while(|(at, grapheme)| at + grapheme.len() <= maximum)
        .last()
        .map_or(0, |(at, grapheme)| at + grapheme.len());
    value.get(..end).unwrap_or_default()
}
