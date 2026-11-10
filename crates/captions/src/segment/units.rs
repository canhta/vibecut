//! Derives indivisible speech units from words, spans or plain text.

use super::TimingKind;
use crate::{Error, LanguageProfile, NfcText, NormalizedText};
use std::{collections::BTreeSet, ops::Range};
use time::{SourceRange, Tick};
use unicode_segmentation::UnicodeSegmentation;

pub(super) struct Unit {
    pub(super) text: String,
    pub(super) range: Range<Tick>,
    pub(super) timing: TimingKind,
    pub(super) boundary: u8,
    pub(super) speaker: Option<NfcText>,
}

pub(super) fn rank(text: &str, profile: &LanguageProfile) -> u8 {
    let end = text
        .trim_end()
        .trim_end_matches(['"', '\'', '”', '’', '」', '』', ')', '）']);
    if end
        .chars()
        .next_back()
        .is_some_and(|c| profile.punctuation.as_str().contains(c))
    {
        3
    } else if end.ends_with([',', ';', ':', '，', '；', '：', '、']) {
        2
    } else {
        0
    }
}

pub(super) fn span_units(
    spans: &crate::TimingSpans,
    profile: &LanguageProfile,
    boundaries: &TextBoundaries,
) -> Result<Option<Vec<Unit>>, Error> {
    let text = spans.display().as_str();
    let mut units: Vec<Unit> = Vec::new();
    let mut offset = 0;
    for group in spans.groups(profile)? {
        let value = text
            .get(group.display.clone())
            .ok_or(Error::Invalid("timing group text"))?;
        if value.trim().is_empty() {
            continue;
        }
        let Some(range) = group.range else {
            return Ok(None);
        };
        let start = group.display.start + value.len() - value.trim_start().len();
        let value = text
            .get(offset..group.display.end)
            .ok_or(Error::Invalid("timing group text"))?;
        if let Some(last) = units
            .last_mut()
            .filter(|_| !boundaries.breaks.contains(&start))
        {
            last.text.push_str(value);
            last.range.end = range.end();
            last.boundary = rank(&last.text, profile);
        } else {
            units.push(Unit {
                text: value.to_owned(),
                range: range.start()..range.end(),
                timing: TimingKind::SourceSpans,
                boundary: rank(value, profile),
                speaker: group.speaker,
            });
        }
        offset = group.display.end;
    }
    if let Some(last) = units.last_mut() {
        last.text.push_str(
            text.get(offset..)
                .ok_or(Error::Invalid("timing group tail"))?,
        );
        last.boundary = rank(&last.text, profile);
        Ok(Some(units))
    } else {
        Ok(None)
    }
}

pub(super) fn timed_units(
    text: &str,
    words: &[crate::Word],
    profile: &LanguageProfile,
    boundaries: &TextBoundaries,
) -> Result<Option<Vec<Unit>>, Error> {
    if words.is_empty()
        || words
            .iter()
            .any(|w| w.range.is_none() || w.text.as_str().trim().is_empty())
    {
        return Ok(None);
    }
    let mut units: Vec<Unit> = Vec::new();
    let mut offset = 0;
    for word in words {
        let suffix = text
            .get(offset..)
            .ok_or(Error::Invalid("word text offset"))?;
        let skipped = suffix.len() - suffix.trim_start().len();
        let value = word.text.as_str().trim();
        let start = offset + skipped;
        let end = start + value.len();
        if text.get(start..end) != Some(value)
            || !boundaries.graphemes.contains(&start)
            || !boundaries.graphemes.contains(&end)
        {
            return Ok(None);
        }
        let Some(range) = word.range else {
            return Ok(None);
        };
        let value = text
            .get(offset..end)
            .ok_or(Error::Invalid("word text offset"))?;
        // UAX #14 places whitespace-separated breaks after the whitespace.
        // Timing tokens can end before it, so test the next visible start.
        if let Some(last) = units
            .last_mut()
            .filter(|_| !boundaries.breaks.contains(&start))
        {
            last.text.push_str(value);
            last.range.end = range.end();
            last.boundary = rank(&last.text, profile);
        } else {
            units.push(Unit {
                text: value.to_owned(),
                range: range.start()..range.end(),
                timing: TimingKind::SourceWords,
                boundary: rank(value, profile),
                speaker: word.speaker.clone(),
            });
        }
        offset = end;
    }
    if !text.get(offset..).is_some_and(|s| s.trim().is_empty()) {
        return Ok(None);
    }
    Ok(Some(units))
}

pub(super) struct TextBoundaries {
    graphemes: BTreeSet<usize>,
    breaks: BTreeSet<usize>,
    total: usize,
}
impl TextBoundaries {
    pub(super) fn new(map: &NormalizedText) -> Self {
        let text = map.text().as_str();
        let graphemes: BTreeSet<_> = std::iter::once(0)
            .chain(map.spans().iter().map(|s| s.normalized.end))
            .collect();
        let breaks: BTreeSet<_> = unicode_linebreak::linebreaks(text)
            .map(|(end, _)| end)
            .filter(|end| graphemes.contains(end))
            .collect();
        Self {
            graphemes,
            breaks,
            total: map.spans().len(),
        }
    }
}

pub(super) fn text_units(
    text: &str,
    range: SourceRange,
    profile: &LanguageProfile,
    boundaries: &TextBoundaries,
    timing: TimingKind,
) -> Result<Vec<Unit>, Error> {
    let mut units = Vec::new();
    let mut start = 0;
    let mut count = 0;
    for &end in &boundaries.breaks {
        let value = text
            .get(start..end)
            .ok_or(Error::Invalid("line break offset"))?;
        let next = count + value.graphemes(true).count();
        if !value.trim().is_empty() {
            units.push(Unit {
                text: value.to_owned(),
                range: proportional(range, count, boundaries.total)?
                    ..proportional(range, next, boundaries.total)?,
                timing,
                boundary: rank(value, profile),
                speaker: None,
            });
        } else if let Some(last) = units.last_mut() {
            last.text.push_str(value);
        }
        start = end;
        count = next;
    }
    Ok(units)
}

fn proportional(range: SourceRange, count: usize, total: usize) -> Result<Tick, Error> {
    if total == 0 {
        return Err(Error::Invalid("empty timing denominator"));
    }
    let offset = i128::from(range.duration().0) * count as i128 / total as i128;
    range
        .start()
        .checked_add(Tick(
            i64::try_from(offset).map_err(|_| Error::Invalid("timing overflow"))?,
        ))
        .ok_or(Error::Invalid("timing overflow"))
}
