use crate::{
    Cue, CueText, Error, LanguageProfile, LanguageTag, NfcText, NormalizedText, Transcript,
    Warning, WarningKind,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};
use time::{FrameRate, SourceRange, TICKS_PER_SECOND, Tick};
use unicode_segmentation::UnicodeSegmentation;

/// Exactly one line-width policy. Pixels require the caller's measurement hook.
#[derive(Clone, Copy, Debug)]
pub enum LineLimit {
    Graphemes(u32),
    Pixels(f64),
}
/// Tick durations, graphemes/second, and a pause threshold (None disables pauses).
/// Conflicting limits warn rather than deleting text or shifting speech timing.
#[derive(Clone, Debug)]
pub struct SegmentRules {
    pub max_lines: u32,
    pub line_limit: LineLimit,
    pub min_duration: Tick,
    pub max_duration: Tick,
    pub max_cps: u32,
    pub min_gap: Tick,
    pub break_on_pause: Option<Tick>,
}
impl SegmentRules {
    /// Adult defaults from the resolved profile, 5/6..7 seconds, two-frame gap.
    pub fn from_profile(profile: &LanguageProfile, rate: FrameRate) -> Result<Self, Error> {
        Ok(Self {
            max_lines: 2,
            line_limit: LineLimit::Graphemes(profile.max_graphemes),
            min_duration: Tick(TICKS_PER_SECOND / 6 * 5),
            max_duration: Tick(TICKS_PER_SECOND * 7),
            max_cps: profile.graphemes_per_second,
            min_gap: rate
                .frame_duration()?
                .checked_add(rate.frame_duration()?)
                .ok_or(Error::Invalid("gap overflow"))?,
            break_on_pause: Some(Tick(TICKS_PER_SECOND)),
        })
    }
    fn validate(&self, hooks: &LayoutHooks<'_>) -> Result<(), Error> {
        let bad_width = match self.line_limit {
            LineLimit::Graphemes(n) => n == 0,
            LineLimit::Pixels(n) => !n.is_finite() || n <= 0.0 || hooks.measure.is_none(),
        };
        if bad_width
            || self.max_lines == 0
            || self.max_cps == 0
            || self.min_duration <= Tick::ZERO
            || self.max_duration < self.min_duration
            || self.min_gap < Tick::ZERO
            || self.break_on_pause.is_some_and(|p| p <= Tick::ZERO)
        {
            return Err(Error::Invalid("segmentation rules"));
        }
        Ok(())
    }
}

type Measure<'a> = dyn Fn(&str) -> Result<f64, Error> + 'a;
type Words<'a> = dyn Fn(&str) -> Result<Vec<Range<usize>>, Error> + 'a;
/// Optional pure callbacks on NFC text. Word ranges must be sorted, nonempty,
/// disjoint and aligned to extended graphemes; uncovered text is still retained.
/// Returned words protect internal UAX#14 break opportunities (e.g. a dictionary
/// segmenter). No dictionary or font engine is bundled by captions.
#[derive(Default)]
pub struct LayoutHooks<'a> {
    pub measure: Option<&'a Measure<'a>>,
    pub words: Option<&'a Words<'a>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimingKind {
    SourceWords,
    SourceSpans,
    SourceSegment,
    Estimated,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegmentedCue {
    pub cue: Cue,
    pub timing: TimingKind,
}
#[derive(Clone, Debug, Default)]
pub struct Segmentation {
    pub cues: Vec<SegmentedCue>,
    pub warnings: Vec<Warning>,
    output_bytes: usize,
}

struct Unit {
    text: String,
    range: Range<Tick>,
    timing: TimingKind,
    boundary: u8,
    speaker: Option<NfcText>,
}
/// Generate fresh cues without editing the transcript. Fully timed word lists
/// must match segment text (apart from whitespace); otherwise segment timing is
/// used with an explicit estimated warning, retaining the authoritative text.
/// Rejects overlapping/negative segments, duplicate/empty IDs and invalid word
/// ranges. Missing word timing uses proportional segment timing, never zero.
pub fn segment_transcript(
    transcript: &Transcript,
    profile: &LanguageProfile,
    rules: &SegmentRules,
    hooks: &LayoutHooks<'_>,
) -> Result<Segmentation, Error> {
    rules.validate(hooks)?;
    let mut output = Segmentation::default();
    validate_transcript(transcript)?;
    for segment in &transcript.segments {
        let normalized = NormalizedText::new(
            segment
                .spans
                .as_ref()
                .map_or(segment.text.as_str(), |s| s.display().as_str()),
        )?;
        let text = normalized.text().as_str();
        if text.trim().is_empty() {
            continue;
        }
        let boundaries = TextBoundaries::new(&normalized, hooks)?;
        let timed = match &segment.spans {
            Some(spans) => span_units(spans, profile, &boundaries)?,
            None => timed_units(text, &segment.words, profile, &boundaries)?,
        };
        let units = match timed {
            Some(units) => units,
            None => text_units(
                text,
                segment.range,
                profile,
                &boundaries,
                if segment.words.is_empty() && segment.spans.is_none() {
                    TimingKind::SourceSegment
                } else {
                    TimingKind::Estimated
                },
            )?,
        };
        append_groups(
            &mut output,
            &units,
            &segment.id,
            &transcript.source_lang,
            segment
                .spans
                .as_ref()
                .and_then(|s| s.display_language())
                .unwrap_or(&transcript.source_lang),
            None,
            false,
            rules,
            hooks,
        )?;
    }
    finish(&mut output, rules)?;
    Ok(output)
}

fn validate_transcript(transcript: &Transcript) -> Result<(), Error> {
    let mut previous = Tick::ZERO;
    let mut ids = BTreeSet::new();
    let mut bytes = 0usize;
    for segment in &transcript.segments {
        bytes = bytes
            .checked_add(segment.id.as_str().len())
            .ok_or(Error::TooLarge)?;
        bytes = bytes
            .checked_add(segment.text.as_str().len())
            .ok_or(Error::TooLarge)?;
        for word in &segment.words {
            bytes = bytes
                .checked_add(word.text.as_str().len())
                .ok_or(Error::TooLarge)?;
        }
        if let Some(spans) = &segment.spans {
            bytes = bytes
                .checked_add(spans.byte_cost())
                .ok_or(Error::TooLarge)?;
        }
        if bytes > 16 * 1024 * 1024 {
            return Err(Error::TooLarge);
        }
        if segment.id.as_str().is_empty()
            || !ids.insert(segment.id.clone())
            || segment.range.start() < previous
        {
            return Err(Error::Invalid("segment identity or order"));
        }
        previous = segment.range.end();
        let mut word_end = segment.range.start();
        for word in &segment.words {
            if let Some(range) = word.range {
                if range.start() < word_end || range.end() > segment.range.end() {
                    return Err(Error::Invalid("word range"));
                }
                word_end = range.end();
            }
        }
        if let Some(spans) = &segment.spans {
            spans.validate_segment(&segment.text, segment.range, &transcript.source_lang)?;
        }
    }
    Ok(())
}

/// An indivisible measured speech unit, or a whole segment when timings are
/// missing/incomplete. Sentence punctuation follows the resolved language profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceSpeechUnit {
    pub range: SourceRange,
    pub sentence_end: bool,
}

/// Return safe source-cut units using the same word/span grouping as subtitles.
/// Never derives cut positions from proportionally estimated text timing.
/// Rejects invalid/overlapping timings and inputs exceeding the segmentation cap.
pub fn source_speech_units(
    transcript: &Transcript,
    profile: &LanguageProfile,
    hooks: &LayoutHooks<'_>,
) -> Result<Vec<SourceSpeechUnit>, Error> {
    if transcript.segments.len() > 10_000
        || transcript
            .segments
            .iter()
            .map(|s| s.words.len())
            .try_fold(0usize, usize::checked_add)
            .is_none_or(|n| n > 500_000)
    {
        return Err(Error::TooLarge);
    }
    validate_transcript(transcript)?;
    let mut output = Vec::new();
    for segment in &transcript.segments {
        let normalized = NormalizedText::new(
            segment
                .spans
                .as_ref()
                .map_or(segment.text.as_str(), |s| s.display().as_str()),
        )?;
        let boundaries = TextBoundaries::new(&normalized, hooks)?;
        let units = match &segment.spans {
            Some(spans) => span_units(spans, profile, &boundaries)?,
            None => timed_units(
                normalized.text().as_str(),
                &segment.words,
                profile,
                &boundaries,
            )?,
        };
        match units {
            Some(units) => {
                if output.len().saturating_add(units.len()) > 500_000 {
                    return Err(Error::TooLarge);
                }
                for unit in units {
                    output.push(SourceSpeechUnit {
                        range: SourceRange::new(unit.range.start, unit.range.end)?,
                        sentence_end: unit.boundary == 3,
                    });
                }
            }
            None => {
                if output.len() >= 500_000 {
                    return Err(Error::TooLarge);
                }
                output.push(SourceSpeechUnit {
                    range: segment.range,
                    sentence_end: rank(normalized.text().as_str(), profile) == 3,
                });
            }
        }
    }
    Ok(output)
}

/// Re-split a translation within exactly one source cue/sentence-group range.
/// Source word timing is never accepted here. Translation edit/reference flags
/// are preserved; every re-split range is proportional and marked Estimated.
/// Generated cues contain target text only; source text remains on the input cue.
pub fn segment_translation(
    source: &Cue,
    target: LanguageTag,
    translation: &CueText,
    profile: &LanguageProfile,
    rules: &SegmentRules,
    hooks: &LayoutHooks<'_>,
) -> Result<Segmentation, Error> {
    rules.validate(hooks)?;
    if source.range.start() < Tick::ZERO || source.id.as_str().is_empty() {
        return Err(Error::Invalid("translation source"));
    }
    for size in [
        source.id.as_str().len(),
        translation
            .translation_ref
            .as_ref()
            .map_or(0, |r| r.as_str().len()),
    ] {
        if size > 16 * 1024 * 1024 {
            return Err(Error::TooLarge);
        }
    }
    let normalized = NormalizedText::new(translation.text.as_str())?;
    let boundaries = TextBoundaries::new(&normalized, hooks)?;
    let units = text_units(
        normalized.text().as_str(),
        source.range,
        profile,
        &boundaries,
        TimingKind::SourceSegment,
    )?;
    let mut output = Segmentation::default();
    append_groups(
        &mut output,
        &units,
        &source.id,
        &source.source_lang,
        &target,
        translation
            .translation_ref
            .clone()
            .or_else(|| Some(source.id.clone())),
        translation.edited,
        rules,
        hooks,
    )?;
    finish(&mut output, rules)?;
    Ok(output)
}

fn rank(text: &str, profile: &LanguageProfile) -> u8 {
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

fn span_units(
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

fn timed_units(
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

struct TextBoundaries {
    graphemes: BTreeSet<usize>,
    breaks: BTreeSet<usize>,
    total: usize,
}
impl TextBoundaries {
    fn new(map: &NormalizedText, hooks: &LayoutHooks<'_>) -> Result<Self, Error> {
        let text = map.text().as_str();
        let boundaries: BTreeSet<_> = std::iter::once(0)
            .chain(map.spans().iter().map(|s| s.normalized.end))
            .collect();
        let protected = match hooks.words {
            Some(words) => words(text)?,
            None => Vec::new(),
        };
        let mut previous = 0;
        for span in &protected {
            if span.start < previous
                || span.start >= span.end
                || !boundaries.contains(&span.start)
                || !boundaries.contains(&span.end)
            {
                return Err(Error::Invalid("word segmenter spans"));
            }
            previous = span.end;
        }
        let breaks: BTreeSet<_> = unicode_linebreak::linebreaks(text)
            .map(|(end, _)| end)
            .filter(|end| {
                boundaries.contains(end)
                    && !protected
                        .get(protected.partition_point(|s| s.end <= *end))
                        .is_some_and(|s| s.start < *end && *end < s.end)
            })
            .collect();
        Ok(Self {
            graphemes: boundaries,
            breaks,
            total: map.spans().len(),
        })
    }
}

fn text_units(
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

fn measure(text: &str, hooks: &LayoutHooks<'_>) -> Result<f64, Error> {
    let width = hooks
        .measure
        .ok_or(Error::Invalid("missing text measurement"))?(text)?;
    if !width.is_finite() || width < 0.0 {
        return Err(Error::Invalid("text measurement"));
    }
    Ok(width)
}

// Layout is incremental over legal units, including long indivisible words.
// Those words remain intact and produce TextLayout warnings instead of truncation.
#[derive(Default)]
struct Lines {
    completed: Vec<String>,
    current: String,
    too_wide: bool,
    graphemes: usize,
    pending_space: bool,
}
impl Lines {
    fn push(
        &mut self,
        unit: &Unit,
        rules: &SegmentRules,
        hooks: &LayoutHooks<'_>,
    ) -> Result<(), Error> {
        let value = unit.text.trim();
        let count = value.graphemes(true).count();
        let separator = !self.current.is_empty()
            && (self.pending_space || unit.text.starts_with(char::is_whitespace));
        let previous_len = self.current.len();
        let candidate_count = self.graphemes + usize::from(separator) + count;
        if separator {
            self.current.push(' ');
        }
        self.current.push_str(value);
        let fits = match rules.line_limit {
            LineLimit::Graphemes(max) => candidate_count as u128 <= u128::from(max),
            LineLimit::Pixels(max) => measure(&self.current, hooks)? <= max,
        };
        if previous_len > 0 && !fits {
            self.current.truncate(previous_len);
            self.completed.push(std::mem::take(&mut self.current));
            self.current.push_str(value);
            self.graphemes = count;
            self.too_wide |= match rules.line_limit {
                LineLimit::Graphemes(max) => count as u128 > u128::from(max),
                LineLimit::Pixels(max) => measure(value, hooks)? > max,
            };
        } else {
            self.graphemes = candidate_count;
            self.too_wide |= !fits;
        }
        self.pending_space = unit.text.ends_with(char::is_whitespace);
        Ok(())
    }
    fn overflow(&self, rules: &SegmentRules) -> bool {
        self.too_wide
            || (self.completed.len() as u128 + u128::from(!self.current.is_empty()))
                > u128::from(rules.max_lines)
    }
}
fn layout(
    units: &[Unit],
    rules: &SegmentRules,
    hooks: &LayoutHooks<'_>,
) -> Result<(String, bool), Error> {
    let mut lines = Lines::default();
    for unit in units {
        lines.push(unit, rules, hooks)?;
    }
    let overflow = lines.overflow(rules);
    if !lines.current.is_empty() {
        lines.completed.push(lines.current.trim().to_owned());
    }
    Ok((lines.completed.join("\n"), overflow))
}

#[allow(
    clippy::too_many_arguments,
    reason = "one internal cue construction boundary"
)]
fn append_groups(
    output: &mut Segmentation,
    units: &[Unit],
    identity: &NfcText,
    source: &LanguageTag,
    language: &LanguageTag,
    reference: Option<NfcText>,
    edited: bool,
    rules: &SegmentRules,
    hooks: &LayoutHooks<'_>,
) -> Result<(), Error> {
    let mut start = 0;
    let initial = output.cues.len();
    while let Some(first) = units.get(start) {
        let mut candidate = Lines::default();
        candidate.push(first, rules, hooks)?;
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
            candidate.push(next, rules, hooks)?;
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
        let (text, overflow) = layout(slice, rules, hooks)?;
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
        output.cues.push(SegmentedCue {
            cue: Cue {
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

fn finish(output: &mut Segmentation, rules: &SegmentRules) -> Result<(), Error> {
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
