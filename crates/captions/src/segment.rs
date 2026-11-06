//! Generates subtitle cues from transcripts.

mod assembly;
mod layout;
mod units;
mod validate;

use std::ops::Range;

use crate::{
    Cue, CueText, Error, LanguageProfile, LanguageTag, NormalizedText, Transcript, Warning,
};
use assembly::{append_groups, finish};
use time::{FrameRate, SourceRange, TICKS_PER_SECOND, Tick};
use units::{TextBoundaries, rank, span_units, text_units, timed_units};
use validate::validate_transcript;

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
#[derive(Clone, Debug, PartialEq)]
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
