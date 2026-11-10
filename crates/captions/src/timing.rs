//! Source timing identity and explicit raw/NFC/display correspondence.
use crate::{Confidence, Error, LanguageProfile, NfcText, NormalizedText, TextSpan};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, ops::Range};
use time::SourceRange;
use unicode_segmentation::UnicodeSegmentation;

const LIMIT: usize = 16 * 1024 * 1024;
/// Native timing identity; grouping never relabels an input unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimingUnit {
    Segment,
    Word,
    Syllable,
    Character,
    Grapheme,
    Token,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimingProvenance {
    Native,
    Forced,
}
/// Adapter input in raw UTF-8 byte coordinates. Index is unique per timing layer
/// within a segment. Optional ranges mean missing timing, never interpolated time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpanTiming {
    pub index: usize,
    pub unit: TimingUnit,
    pub raw: Range<usize>,
    pub range: Option<SourceRange>,
    pub provenance: TimingProvenance,
    pub speaker: Option<NfcText>,
    pub confidence: Option<Confidence>,
}
/// Caller-resolved same-language display variant (e.g. Han phrase conversion).
/// Source endpoints are NFC grapheme boundaries; replacement is nonempty NFC.
/// These correspondences are not translation word alignment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayReplacement {
    pub normalized: Range<usize>,
    pub text: NfcText,
}
/// Selected same-language script/orthography output and explicit phrase mapping.
/// Callers provide reviewed conversion data; this crate bundles no Han lexicon.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayVariant {
    pub language: crate::LanguageTag,
    pub replacements: Vec<DisplayReplacement>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct MappedTiming {
    input: SpanTiming,
    normalized: Range<usize>,
    display: Range<usize>,
}
impl MappedTiming {
    pub fn input(&self) -> &SpanTiming {
        &self.input
    }
    pub fn normalized(&self) -> Range<usize> {
        self.normalized.clone()
    }
    pub fn display(&self) -> Range<usize> {
        self.display.clone()
    }
}
/// Grouping view of one source layer, retaining its contributing ordinal IDs.
/// A missing input range or uncovered visible grapheme leaves the group untimed.
#[derive(Clone, Debug, PartialEq)]
pub struct TimingGroup {
    pub display: Range<usize>,
    pub range: Option<SourceRange>,
    pub indices: Vec<usize>,
    pub speaker: Option<NfcText>,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "StoredSpans", into = "StoredSpans")]
pub struct TimingSpans {
    raw: String,
    normalized: NormalizedText,
    display: NfcText,
    correspondence: Vec<TextSpan>,
    variant: Option<DisplayVariant>,
    spans: Vec<MappedTiming>,
    byte_cost: usize,
}
impl std::fmt::Debug for TimingSpans {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimingSpans")
            .field("spans", &self.spans)
            .finish_non_exhaustive()
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredSpans {
    raw: String,
    spans: Vec<SpanTiming>,
    variant: Option<DisplayVariant>,
}
impl TryFrom<StoredSpans> for TimingSpans {
    type Error = Error;
    fn try_from(value: StoredSpans) -> Result<Self, Error> {
        Self::new(&value.raw, value.spans, value.variant)
    }
}
impl From<TimingSpans> for StoredSpans {
    fn from(value: TimingSpans) -> Self {
        Self {
            raw: value.raw,
            spans: value.spans.into_iter().map(|s| s.input).collect(),
            variant: value.variant,
        }
    }
}
impl TimingSpans {
    /// Validate raw text, independent ordered layers and explicit display edits.
    /// Total stored text/map/input overhead is bounded to 16 MiB. Raw token bytes
    /// must concatenate to valid UTF-8 before this boundary; individual spans may
    /// split scalars. No timing is inferred from byte or character length.
    pub fn new(
        raw: &str,
        inputs: Vec<SpanTiming>,
        variant: Option<DisplayVariant>,
    ) -> Result<Self, Error> {
        let replacements = variant
            .as_ref()
            .map_or(&[][..], |v| v.replacements.as_slice());
        let size = inputs
            .len()
            .checked_mul(std::mem::size_of::<MappedTiming>())
            .and_then(|n| {
                n.checked_add(
                    replacements
                        .len()
                        .checked_mul(std::mem::size_of::<DisplayReplacement>())?,
                )
            })
            .and_then(|n| n.checked_add(raw.len()))
            .ok_or(Error::TooLarge)?;
        if size > LIMIT {
            return Err(Error::TooLarge);
        }
        let normalized = NormalizedText::new(raw)?;
        let (display, correspondence) = display_map(&normalized, replacements)?;
        let overhead = normalized
            .spans()
            .len()
            .checked_add(correspondence.len())
            .and_then(|n| n.checked_mul(std::mem::size_of::<TextSpan>()))
            .and_then(|n| n.checked_add(size))
            .and_then(|n| n.checked_add(display.as_str().len()))
            .and_then(|n| n.checked_add(normalized.text().as_str().len()))
            .ok_or(Error::TooLarge)?;
        let extra = replacements.iter().try_fold(0usize, |sum, r| {
            sum.checked_add(r.text.as_str().len())
                .ok_or(Error::TooLarge)
        })?;
        let metadata = inputs.iter().try_fold(0usize, |sum, span| {
            sum.checked_add(span.speaker.as_ref().map_or(0, |s| s.as_str().len()))
                .ok_or(Error::TooLarge)
        })?;
        let byte_cost = overhead
            .checked_add(extra)
            .and_then(|n| n.checked_add(metadata))
            .ok_or(Error::TooLarge)?;
        if byte_cost > LIMIT {
            return Err(Error::TooLarge);
        }
        let mut layers = BTreeMap::new();
        let mut spans = Vec::with_capacity(inputs.len());
        for input in inputs {
            let normalized_range = normalized
                .normalized_range(input.raw.clone())
                .ok_or(Error::Invalid("raw timing span"))?;
            let display_range = mapped_range(&correspondence, normalized_range.clone())
                .ok_or(Error::Invalid("display timing span"))?;
            let (last_start, last_end, last_time, last_index) =
                layers.entry(input.unit).or_insert((0, 0, None, None));
            if input.raw.start < *last_start
                || input.raw.end < *last_end
                || last_index.is_some_and(|i| input.index <= i)
                || input.range.is_some_and(|r| {
                    r.start() < time::Tick::ZERO || last_time.is_some_and(|t| r.start() < t)
                })
            {
                return Err(Error::Invalid("timing layer order"));
            }
            *last_start = input.raw.start;
            *last_end = input.raw.end;
            *last_index = Some(input.index);
            if let Some(range) = input.range {
                *last_time = Some(range.end());
            }
            spans.push(MappedTiming {
                input,
                normalized: normalized_range,
                display: display_range,
            });
        }
        Ok(Self {
            raw: raw.to_owned(),
            normalized,
            display,
            correspondence,
            variant,
            spans,
            byte_cost,
        })
    }
    /// Rebuild display correspondence from the original raw source, retaining all
    /// source ranges and identities. Never chain converted display byte offsets.
    pub fn with_display(&self, variant: DisplayVariant) -> Result<Self, Error> {
        Self::new(
            &self.raw,
            self.spans.iter().map(|s| s.input.clone()).collect(),
            Some(variant),
        )
    }
    pub fn display_language(&self) -> Option<&crate::LanguageTag> {
        self.variant.as_ref().map(|v| &v.language)
    }
    pub(crate) fn byte_cost(&self) -> usize {
        self.byte_cost
    }
    pub fn raw(&self) -> &str {
        &self.raw
    }
    pub fn normalized(&self) -> &NormalizedText {
        &self.normalized
    }
    pub fn display(&self) -> &NfcText {
        &self.display
    }
    /// NFC -> display byte correspondence. Each entry may cover many graphemes.
    pub fn correspondence(&self) -> &[TextSpan] {
        &self.correspondence
    }
    pub fn spans(&self) -> &[MappedTiming] {
        &self.spans
    }
    pub(crate) fn validate_segment(
        &self,
        text: &NfcText,
        range: SourceRange,
        language: &crate::LanguageTag,
    ) -> Result<(), Error> {
        if self
            .variant
            .as_ref()
            .is_some_and(|v| v.language.primary() != language.primary())
        {
            return Err(Error::Invalid("display variant changes source language"));
        }
        if self.normalized.text() != text
            || self.spans.iter().any(|s| {
                s.input
                    .range
                    .is_some_and(|r| r.start() < range.start() || r.end() > range.end())
            })
        {
            return Err(Error::Invalid("timing spans differ from segment"));
        }
        Ok(())
    }
    /// Select a layer deterministically: profile preference, tokens, graphemes,
    /// characters, syllables, words, segments. UAX #29 keeps Latin/full-width runs
    /// together and separates Han. A provider span or display phrase crossing a
    /// candidate boundary joins the groups: native timings are never subdivided.
    pub fn groups(&self, profile: &LanguageProfile) -> Result<Vec<TimingGroup>, Error> {
        let selected = [
            profile.timing_unit,
            TimingUnit::Token,
            TimingUnit::Grapheme,
            TimingUnit::Character,
            TimingUnit::Syllable,
            TimingUnit::Word,
            TimingUnit::Segment,
        ]
        .into_iter()
        .find(|u| self.spans.iter().any(|s| s.input.unit == *u));
        let Some(selected) = selected else {
            return Ok(Vec::new());
        };
        let layer: Vec<_> = self
            .spans
            .iter()
            .filter(|s| s.input.unit == selected)
            .collect();
        let text = self.display.as_str();
        let mut forbidden = Vec::with_capacity(layer.len() + self.correspondence.len());
        forbidden.extend(layer.iter().map(|s| s.display.clone()));
        forbidden.extend(self.correspondence.iter().map(|s| s.normalized.clone()));
        forbidden.sort_by_key(|r| r.start);
        let mut ranges = Vec::<Range<usize>>::new();
        let mut start = 0;
        let mut cursor = 0;
        let mut protected_end = 0;
        // UAX #29 boundaries are grapheme-safe. Token preference retains actual
        // token boundaries when they coincide with whole display graphemes.
        let ends: Vec<_> = if profile.timing_unit == TimingUnit::Token {
            self.correspondence
                .iter()
                .map(|s| s.normalized.end)
                .collect()
        } else {
            text.split_word_bound_indices()
                .map(|(i, s)| i + s.len())
                .collect()
        };
        for end in ends {
            while let Some(r) = forbidden.get(cursor).filter(|r| r.start < end) {
                protected_end = protected_end.max(r.end);
                cursor += 1;
            }
            if protected_end > end {
                continue;
            }
            ranges.push(start..end);
            start = end;
        }
        let mut output = Vec::with_capacity(ranges.len());
        let mut cursor = 0;
        for display in ranges {
            while layer
                .get(cursor)
                .is_some_and(|s| s.display.end <= display.start)
            {
                cursor += 1;
            }
            let mut end = cursor;
            let first = self
                .correspondence
                .partition_point(|s| s.normalized.end <= display.start);
            let last = self
                .correspondence
                .partition_point(|s| s.normalized.start < display.end);
            let nfc = self
                .correspondence
                .get(first)
                .ok_or(Error::Invalid("group correspondence"))?
                .raw
                .start
                ..self
                    .correspondence
                    .get(
                        last.checked_sub(1)
                            .ok_or(Error::Invalid("group correspondence"))?,
                    )
                    .ok_or(Error::Invalid("group correspondence"))?
                    .raw
                    .end;
            let raw = self
                .normalized
                .raw_range(nfc)
                .ok_or(Error::Invalid("group raw correspondence"))?;
            let mut covered = raw.start;
            let mut timed = true;
            let mut range: Option<SourceRange> = None;
            let mut indices = Vec::new();
            let mut speaker = None;
            while let Some(span) = layer.get(end).filter(|s| s.display.start < display.end) {
                if span.input.raw.start > covered
                    && !self
                        .raw
                        .get(covered..span.input.raw.start)
                        .is_some_and(|s| s.trim().is_empty())
                {
                    timed = false;
                }
                covered = covered.max(span.input.raw.end);
                if let Some(r) = span.input.range {
                    range = Some(match range {
                        Some(previous) => SourceRange::new(previous.start(), r.end())?,
                        None => r,
                    });
                } else {
                    timed = false;
                }
                if indices.is_empty() {
                    speaker = span.input.speaker.clone();
                }
                indices.push(span.input.index);
                end += 1;
            }
            if covered < raw.end
                && !self
                    .raw
                    .get(covered..raw.end)
                    .is_some_and(|s| s.trim().is_empty())
            {
                timed = false;
            }
            output.push(TimingGroup {
                display,
                range: if timed { range } else { None },
                indices,
                speaker,
            });
        }
        Ok(output)
    }
}
// TextSpan names describe the two sides: raw=NFC, normalized=display here.
fn mapped_range(map: &[TextSpan], range: Range<usize>) -> Option<Range<usize>> {
    let start = map.partition_point(|s| s.raw.end <= range.start);
    let end = map.partition_point(|s| s.raw.start < range.end);
    Some(map.get(start)?.normalized.start..map.get(end.checked_sub(1)?)?.normalized.end)
}
fn display_map(
    source: &NormalizedText,
    replacements: &[DisplayReplacement],
) -> Result<(NfcText, Vec<TextSpan>), Error> {
    let boundary = |i| {
        i == 0
            || source
                .spans()
                .binary_search_by_key(&i, |s| s.normalized.end)
                .is_ok()
    };
    let mut previous = 0;
    let mut replacement_bytes = 0usize;
    for r in replacements {
        replacement_bytes = replacement_bytes
            .checked_add(r.text.as_str().len())
            .ok_or(Error::TooLarge)?;
        if replacement_bytes > LIMIT {
            return Err(Error::TooLarge);
        }
        if r.normalized.start < previous
            || r.normalized.start >= r.normalized.end
            || !boundary(r.normalized.start)
            || !boundary(r.normalized.end)
            || r.text.as_str().is_empty()
        {
            return Err(Error::Invalid("display replacement"));
        }
        previous = r.normalized.end;
    }
    let mut display = String::new();
    let mut map = Vec::new();
    let mut cursor = 0;
    let mut replacement = 0;
    while let Some(span) = source.spans().get(cursor) {
        let (range, text) = if let Some(r) = replacements
            .get(replacement)
            .filter(|r| r.normalized.start == span.normalized.start)
        {
            replacement += 1;
            (r.normalized.clone(), r.text.as_str())
        } else {
            (
                span.normalized.clone(),
                source
                    .text()
                    .as_str()
                    .get(span.normalized.clone())
                    .ok_or(Error::Invalid("display source"))?,
            )
        };
        let start = display.len();
        if start.checked_add(text.len()).is_none_or(|n| n > LIMIT) {
            return Err(Error::TooLarge);
        }
        display.push_str(text);
        map.push(TextSpan {
            raw: range.clone(),
            normalized: start..display.len(),
        });
        cursor = source
            .spans()
            .partition_point(|s| s.normalized.end <= range.end);
    }
    // An edit must not compose/resegment across its neighbors; such a phrase
    // must be supplied as one larger correspondence by the caller.
    let text = NfcText::from(display.as_str());
    if text.as_str() != display
        || map.iter().any(|s| {
            !display.is_char_boundary(s.normalized.start)
                || !display.is_char_boundary(s.normalized.end)
        })
    {
        return Err(Error::Invalid("display normalization"));
    }
    let boundaries: std::collections::BTreeSet<_> = std::iter::once(0)
        .chain(display.grapheme_indices(true).map(|(i, s)| i + s.len()))
        .collect();
    if map.iter().any(|s| {
        !boundaries.contains(&s.normalized.start) || !boundaries.contains(&s.normalized.end)
    }) {
        return Err(Error::Invalid("display grapheme boundary"));
    }
    Ok((text, map))
}
