//! Turns speech units into positioned cues.

use super::layout::{Lines, layout};
use super::units::Unit;
use super::{SegmentRules, Segmentation, SegmentedCue, TimingKind};
use crate::{Cue, CueText, Error, LanguageTag, NfcText, Warning, WarningKind};
use std::collections::BTreeMap;
use time::{SourceRange, TICKS_PER_SECOND};
use unicode_segmentation::UnicodeSegmentation;

#[allow(
    clippy::too_many_arguments,
    reason = "one internal cue construction boundary"
)]
pub(super) fn append_groups(
    output: &mut Segmentation,
    units: &[Unit],
    identity: &NfcText,
    source: &LanguageTag,
    language: &LanguageTag,
    reference: Option<NfcText>,
    edited: bool,
    rules: &SegmentRules,
) -> Result<(), Error> {
    let mut start = 0;
    let initial = output.cues.len();
    while let Some(first) = units.get(start) {
        let mut candidate = Lines::default();
        candidate.push(first, rules);
        let mut end = start + 1;
        let mut preferred = None;
        let mut priority = 0;
        while let Some(next) = units.get(end) {
            let previous = units.get(end - 1).ok_or(Error::Invalid("unit offset"))?;
            let pause = rules
                .break_on_pause
                .is_some_and(|p| next.range.start - previous.range.end >= p);
            let boundary = previous.boundary.max(u8::from(pause));
            if boundary > 0 && boundary >= priority {
                preferred = Some(end);
                priority = boundary;
            }
            candidate.push(next, rules);
            if next.speaker != first.speaker
                || pause
                || next.range.end - first.range.start > rules.max_duration
                || candidate.overflow(rules)
            {
                break;
            }
            end += 1;
        }
        // On a capacity boundary, use the strongest sentence/clause/pause break.
        if end < units.len() {
            end = preferred.unwrap_or(end);
        }
        let slice = units.get(start..end).ok_or(Error::Invalid("unit offset"))?;
        let last = slice.last().ok_or(Error::Invalid("empty cue"))?;
        let (text, overflow) = layout(slice, rules);
        let timing = if slice.iter().all(|u| u.timing == TimingKind::SourceWords) {
            TimingKind::SourceWords
        } else if slice.iter().all(|u| u.timing == TimingKind::SourceSpans) {
            TimingKind::SourceSpans
        } else if start == 0
            && end == units.len()
            && slice.iter().all(|u| u.timing != TimingKind::Estimated)
        {
            TimingKind::SourceSegment
        } else {
            TimingKind::Estimated
        };
        // Bound repeated identities/references and per-cue/warning overhead,
        // not just input text: narrow rules must not amplify a large ID freely.
        let costs = [
            std::mem::size_of::<SegmentedCue>(),
            5 * std::mem::size_of::<Warning>(),
            identity.as_str().len(),
            language.as_str().len() * 2,
            source.as_str().len(),
            reference.as_ref().map_or(0, |r| r.as_str().len()),
            text.len(),
            if reference.is_none()
                && !edited
                && slice.iter().all(|unit| unit.word_provenance.is_some())
            {
                text.len()
                    .checked_add(
                        slice
                            .len()
                            .checked_mul(std::mem::size_of::<crate::VerifiedCueWord>())
                            .ok_or(Error::TooLarge)?,
                    )
                    .and_then(|bytes| bytes.checked_add(language.as_str().len()))
                    .ok_or(Error::TooLarge)?
            } else {
                0
            },
            64,
        ];
        output.output_bytes = costs
            .into_iter()
            .try_fold(output.output_bytes, |total, cost| {
                total
                    .checked_add(cost)
                    .filter(|size| *size <= 16 * 1024 * 1024)
                    .ok_or(Error::TooLarge)
            })?;
        let position = output.cues.len() + 1;
        if overflow {
            output.warnings.push(Warning {
                position,
                kind: WarningKind::TextLayout,
            });
        }
        // Only measured ASR units become word timings; estimated/proportional
        // ranges stay absent so callers never treat them as native.
        let word_ranges = if slice.iter().all(|unit| {
            matches!(
                unit.timing,
                TimingKind::SourceWords | TimingKind::SourceSpans
            )
        }) {
            slice
                .iter()
                .map(|unit| SourceRange::new(unit.range.start, unit.range.end))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        output.cues.push(SegmentedCue {
            cue: Cue {
                word_timing: correspondence(slice, &text, language, edited, reference.is_some())?,
                id: format!(
                    "{}:{}:{}:{}",
                    identity.as_str().len(),
                    identity.as_str(),
                    language.as_str(),
                    output.cues.len() - initial
                )
                .into(),
                range: SourceRange::new(first.range.start, last.range.end)?,
                source_lang: source.clone(),
                texts: BTreeMap::from([(
                    language.clone(),
                    CueText {
                        text: text.into(),
                        edited,
                        translation_ref: reference.clone(),
                    },
                )]),
                word_ranges,
                style: None,
            },
            timing,
        });
        start = end;
    }
    // A split translation is always estimated, including the final fragment.
    if reference.is_some() && output.cues.len() - initial > 1 {
        for cue in output.cues.iter_mut().skip(initial) {
            cue.timing = TimingKind::Estimated;
        }
    }
    Ok(())
}

pub(super) fn finish(output: &mut Segmentation, rules: &SegmentRules) -> Result<(), Error> {
    for i in 0..output.cues.len() {
        let next_start = output.cues.get(i + 1).map(|c| c.cue.range.start());
        let entry = output.cues.get_mut(i).ok_or(Error::Invalid("cue offset"))?;
        let start = entry.cue.range.start();
        let mut end = entry.cue.range.end();
        // Source word cues may borrow only actual following silence, with no
        // invented last-cue extension. Estimated/segment cues stay inside input.
        if matches!(
            entry.timing,
            TimingKind::SourceWords | TimingKind::SourceSpans
        ) && end - start < rules.min_duration
            && let Some(next) = next_start
        {
            let desired = start
                .checked_add(rules.min_duration)
                .ok_or(Error::Invalid("duration overflow"))?;
            end = end.max(desired.min(next.saturating_sub(rules.min_gap)));
            entry.cue.range = SourceRange::new(start, end)?;
        }
        let position = i + 1;
        let mut warn = |kind| output.warnings.push(Warning { position, kind });
        if end - start < rules.min_duration || end - start > rules.max_duration {
            warn(WarningKind::Duration);
        }
        if next_start.is_some_and(|next| next - end < rules.min_gap) {
            warn(WarningKind::MinimumGap);
        }
        if entry.timing == TimingKind::Estimated {
            warn(WarningKind::EstimatedTiming);
        }
        let count: usize = entry
            .cue
            .texts
            .values()
            .map(|t| {
                t.text
                    .as_str()
                    .graphemes(true)
                    .filter(|g| *g != "\n")
                    .count()
            })
            .sum();
        if count as i128 * i128::from(TICKS_PER_SECOND)
            > i128::from(rules.max_cps) * i128::from((end - start).0)
        {
            warn(WarningKind::ReadingSpeed);
        }
    }
    Ok(())
}

fn correspondence(
    units: &[Unit],
    text: &str,
    language: &LanguageTag,
    edited: bool,
    translation: bool,
) -> Result<crate::CueWordTiming, Error> {
    if edited || translation {
        return Ok(crate::CueWordTiming::Unverified);
    }
    let mut records = Vec::new();
    let mut cursor = 0;
    for unit in units {
        let Some(provenance) = unit.word_provenance else {
            return Ok(crate::CueWordTiming::Unverified);
        };
        let suffix = text
            .get(cursor..)
            .ok_or(Error::Invalid("cue word correspondence"))?;
        let start = cursor + suffix.len() - suffix.trim_start().len();
        let end = start
            .checked_add(unit.text.trim().len())
            .ok_or(Error::TooLarge)?;
        if text.get(start..end) != Some(unit.text.trim()) {
            return Ok(crate::CueWordTiming::Unverified);
        }
        records.push(crate::VerifiedCueWord {
            bytes: start..end,
            source_range: SourceRange::new(unit.range.start, unit.range.end)?,
            projection: crate::WordProjection::Visible(SourceRange::new(
                unit.range.start,
                unit.range.end,
            )?),
            provenance,
        });
        cursor = end;
    }
    if !text
        .get(cursor..)
        .is_some_and(|tail| tail.trim().is_empty())
        || records.is_empty()
    {
        return Ok(crate::CueWordTiming::Unverified);
    }
    let value = crate::CueWordTiming::Words {
        language: language.clone(),
        text: text.into(),
        records,
    };
    value.validate()?;
    Ok(value)
}
