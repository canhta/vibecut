use crate::{Error, NfcText};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// Corresponding UTF-8 byte ranges for a whole extended grapheme before/after NFC.
/// Several raw scalars can correspond to one normalized scalar; offsets inside a
/// grapheme are intentionally not represented as a one-to-one mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSpan {
    pub raw: Range<usize>,
    pub normalized: Range<usize>,
}

/// Captions-owned normalization and grapheme correspondence, reusable by adapters.
/// Input is capped at 16 MiB. Debug redacts text; maps contain byte offsets only.
#[derive(Clone, Debug)]
pub struct NormalizedText {
    text: NfcText,
    spans: Vec<TextSpan>,
}
impl NormalizedText {
    pub fn new(raw: &str) -> Result<Self, Error> {
        if raw.len() > 16 * 1024 * 1024 {
            return Err(Error::TooLarge);
        }
        let mut text = String::with_capacity(raw.len());
        let mut spans = Vec::new();
        for (start, grapheme) in raw.grapheme_indices(true) {
            let normalized = NfcText::from(grapheme);
            let offset = text.len();
            text.push_str(normalized.as_str());
            spans.push(TextSpan {
                raw: start..start + grapheme.len(),
                normalized: offset..text.len(),
            });
        }
        Ok(Self {
            text: text.into(),
            spans,
        })
    }
    pub fn text(&self) -> &NfcText {
        &self.text
    }
    pub fn spans(&self) -> &[TextSpan] {
        &self.spans
    }
    /// Enclosing raw graphemes for a nonempty normalized byte range. Invalid,
    /// empty and out-of-bounds ranges return None; partial graphemes expand.
    pub fn raw_range(&self, normalized: Range<usize>) -> Option<Range<usize>> {
        if normalized.start >= normalized.end || normalized.end > self.text.as_str().len() {
            return None;
        }
        let start = self
            .spans
            .partition_point(|s| s.normalized.end <= normalized.start);
        let end = self
            .spans
            .partition_point(|s| s.normalized.start < normalized.end);
        Some(self.spans.get(start)?.raw.start..self.spans.get(end.checked_sub(1)?)?.raw.end)
    }
}
