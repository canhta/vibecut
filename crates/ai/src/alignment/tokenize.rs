use crate::{Error, RecognitionSegment, TimingUnit};
use captions::{LanguageProfile, NormalizedText, SpanTiming, TimingSpans};
use std::ops::Range;
use tokenizers::Tokenizer;
/// One profile group and its model-token extent; whitespace stays in input only.
pub(super) struct Unit {
    pub segment: usize,
    pub text: Range<usize>,
    pub tokens: Range<usize>,
    pub kind: TimingUnit,
}
pub(super) fn encode(
    tokenizer: &Tokenizer,
    segments: &[RecognitionSegment],
    profile: &LanguageProfile,
    eot: u32,
) -> Result<(Vec<u32>, Vec<Unit>), Error> {
    let mut ids = Vec::new();
    let mut units = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        let text = segment.text.as_str();
        let normalized = NormalizedText::new(text)?;
        let inputs = normalized
            .spans()
            .iter()
            .enumerate()
            .map(|(index, s)| SpanTiming {
                index,
                unit: TimingUnit::Grapheme,
                raw: s.raw.clone(),
                range: None,
                provenance: captions::TimingProvenance::Forced,
                speaker: None,
                confidence: None,
            })
            .collect();
        let spans = TimingSpans::new(text, inputs, None)?;
        let mut pending = 0;
        for group in spans.groups(profile)? {
            let part = text
                .get(group.display.clone())
                .ok_or(Error::Invalid("alignment text grouping"))?;
            if part.trim().is_empty() {
                continue;
            }
            let range = pending..group.display.end;
            let value = text.get(range.clone()).ok_or(Error::Inference)?;
            let encoded = tokenizer
                .encode(value, false)
                .map_err(|_| Error::Invalid("alignment tokenizer"))?;
            let start = ids.len();
            if encoded.get_ids().iter().any(|id| *id >= eot) {
                return Err(Error::Invalid("special token in alignment text"));
            }
            ids.extend_from_slice(encoded.get_ids());
            if ids.len() > 443 {
                return Err(Error::Payload);
            }
            let trim_start = range.start + value.len() - value.trim_start().len();
            let trim_end = range.end - (value.len() - value.trim_end().len());
            if value.chars().any(char::is_alphanumeric) {
                units.push(Unit {
                    segment: index,
                    text: trim_start..trim_end,
                    tokens: start..ids.len(),
                    kind: profile.timing_unit,
                });
            }
            pending = group.display.end;
        }
        // Trailing spaces remain teacher-forcing input, not a timed lexical unit.
        if pending < text.len() {
            let tail = text.get(pending..).ok_or(Error::Inference)?;
            let e = tokenizer
                .encode(tail, false)
                .map_err(|_| Error::Invalid("alignment tokenizer"))?;
            if e.get_ids().iter().any(|id| *id >= eot) {
                return Err(Error::Invalid("special token in alignment text"));
            }
            ids.extend_from_slice(e.get_ids());
        }
        if index + 1 < segments.len() {
            let separator = tokenizer
                .encode(" ", false)
                .map_err(|_| Error::Invalid("alignment tokenizer"))?;
            ids.extend_from_slice(separator.get_ids());
        }
        if ids.len() > 443 {
            return Err(Error::Payload);
        }
    }
    if units.is_empty() || ids.is_empty() {
        return Err(Error::TimingUnavailable);
    }
    Ok((ids, units))
}
