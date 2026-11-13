//! Builds, encodes and checks the per-asset transcript index entry.
use super::Segment;
use super::folding::fold;
use crate::{EditError, transcript::Error, transcript::Result};
use serde::{Deserialize, Serialize};
/// Index format and folding rules. A change here misses every older entry.
pub(super) const VERSION: &str = "format-1;fold-1";
#[derive(Serialize, Deserialize)]
pub(super) struct Entry {
    pub(super) start: i64,
    pub(super) end: i64,
    pub(super) text: String,
    pub(super) folded: String,
}
#[derive(Serialize, Deserialize)]
pub(super) struct Index {
    version: String,
    content: [u8; 32],
    fingerprint: [u8; 32],
    pub(super) entries: Vec<Entry>,
}
impl Index {
    /// True when this entry was built from the same media content and transcript.
    pub(super) fn matches(&self, content: [u8; 32], fingerprint: [u8; 32]) -> bool {
        self.version == VERSION && self.content == content && self.fingerprint == fingerprint
    }
}
/// Identity of one media content and transcript: any change to either changes it.
pub(super) fn fingerprint(content: [u8; 32], segments: &[Segment]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&content);
    for segment in segments {
        hasher.update(&segment.start.0.to_le_bytes());
        hasher.update(&segment.end.0.to_le_bytes());
        hasher.update(
            &u64::try_from(segment.text.len())
                .unwrap_or(u64::MAX)
                .to_le_bytes(),
        );
        hasher.update(segment.text.as_bytes());
    }
    *hasher.finalize().as_bytes()
}
pub(super) fn build(content: [u8; 32], fingerprint: [u8; 32], segments: &[Segment]) -> Index {
    Index {
        version: VERSION.to_owned(),
        content,
        fingerprint,
        entries: segments
            .iter()
            .map(|segment| Entry {
                start: segment.start.0,
                end: segment.end.0,
                text: segment.text.clone(),
                folded: fold(&segment.text),
            })
            .collect(),
    }
}
pub(super) fn encode(index: &Index) -> Result<Vec<u8>> {
    serde_json::to_vec(index)
        .map_err(|_| Error::Edit(EditError::Invalid("transcript index encoding failed")))
}
/// Unreadable bytes decode to `None`, which the caller treats as a miss.
pub(super) fn decode(bytes: &[u8]) -> Option<Index> {
    serde_json::from_slice(bytes).ok()
}
