//! The bundle manifest, its fixed layout and the closed media file-name rule.
use media::AssetId;
use serde::{Deserialize, Serialize};

pub const BUNDLE_FORMAT_ID: &str = "vibecut.bundle";
/// Largest single media copy a bundle carries or accepts.
pub const MAX_MEDIA_BYTES: u64 = 64 << 30;
/// Largest total of distinct media copies in one bundle.
pub const MAX_BUNDLE_BYTES: u64 = 512 << 30;

pub(super) const MANIFEST_FILE: &str = "bundle.json";
pub(super) const PROJECT_FILE: &str = "project.vibecut";
pub(super) const MEDIA_DIR: &str = "media";
pub(super) const MAX_MANIFEST_BYTES: u64 = 4 << 20;
const MAX_NAME_BYTES: usize = 80;
const FALLBACK_NAME: &str = "media";

/// One entry per project asset. The stored file is never named by the manifest:
/// it is always `media/<hash>-<name>`, so no manifest text is ever a path.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub format: String,
    pub schema_version: u32,
    pub generator: String,
    pub media: Vec<Entry>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub asset: AssetId,
    /// The original file name, reduced to the closed name rule.
    pub name: String,
    /// Lowercase hex BLAKE3 of the media bytes.
    pub hash: String,
    /// Copied byte count; zero when the media was missing at export.
    pub bytes: u64,
}

/// The stored file name for content `hash` (lowercase hex) and a valid `name`.
pub(super) fn stored_name(hash: &str, name: &str) -> String {
    format!("{hash}-{name}")
}

/// Reduce an original file name to the closed rule: no controls, separators or
/// Windows-reserved punctuation, no trailing dots or spaces, at most 80 bytes,
/// never empty. Idempotent, so `valid_name` is `sanitize(name) == name`.
pub(super) fn sanitize(raw: &str) -> String {
    let mapped: String = raw
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let mut name = String::new();
    for c in mapped.trim_end_matches(['.', ' ']).chars() {
        if name.len() + c.len_utf8() > MAX_NAME_BYTES {
            break;
        }
        name.push(c);
    }
    let name = name.trim_end_matches(['.', ' ']);
    if name.is_empty() {
        FALLBACK_NAME.to_owned()
    } else {
        name.to_owned()
    }
}

pub(super) fn valid_name(name: &str) -> bool {
    sanitize(name) == name
}

pub(super) fn hex(bytes: &[u8; 32]) -> String {
    use std::fmt::Write;
    let mut text = String::with_capacity(64);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// Exactly 64 lowercase hex digits.
pub(super) fn parse_hex(text: &str) -> Option<[u8; 32]> {
    fn digit(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        }
    }
    if text.len() != 64 {
        return None;
    }
    let mut bytes = Vec::with_capacity(32);
    for pair in text.as_bytes().chunks_exact(2) {
        let [high, low] = pair else {
            return None;
        };
        bytes.push((digit(*high)? << 4) | digit(*low)?);
    }
    <[u8; 32]>::try_from(bytes).ok()
}
