use crate::contracts::checkpoint;
use crate::{
    Aligner, AudioWindow, Error, ExecutionHooks, Operation, Request, SampleRange, Stage,
    TimingUnit, Transcriber,
};
use captions::{Fingerprint, LanguageTag, NfcText, Segment, Transcript, Word};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use time::{Rounding, SourceRange, Tick};

/// Adapter transport data, not a second persisted caption model. Non-word timing
/// units remain explicit and are never relabelled as words during conversion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimedText {
    pub text: NfcText,
    pub unit: TimingUnit,
    pub range: SampleRange,
    pub provenance: Provenance,
    #[serde(default)]
    pub speaker: Option<NfcText>,
    #[serde(default)]
    pub confidence: Option<captions::Confidence>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    Native,
    Forced,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecognitionSegment {
    pub id: NfcText,
    pub text: NfcText,
    pub range: Option<SampleRange>,
    pub timing: Vec<TimedText>,
}
impl RecognitionSegment {
    fn validate(&self, window: SampleRange) -> Result<(), Error> {
        crate::policy::nonempty(&self.id)?;
        if self.range.is_some_and(|r| !window.contains(r)) {
            return Err(Error::Invalid("segment outside audio"));
        }
        if self.range.is_none() && !self.timing.is_empty() {
            return Err(Error::Invalid("unit timing without segment timing"));
        }
        let mut cursors = BTreeMap::new();
        for span in &self.timing {
            let (cursor, previous) = cursors.entry(span.unit).or_insert((0usize, None));
            if span.text.as_str().is_empty() || !self.range.is_some_and(|r| r.contains(span.range))
            {
                return Err(Error::Invalid("unit outside segment"));
            }
            if previous.is_some_and(|end| span.range.start() < end) {
                return Err(Error::Invalid("nonmonotonic unit timing"));
            }
            let remaining = self
                .text
                .as_str()
                .get(*cursor..)
                .ok_or(Error::Invalid("text boundary"))?;
            let offset = remaining
                .find(span.text.as_str())
                .ok_or(Error::Invalid("unit text differs from recognition"))?;
            *cursor = cursor
                .checked_add(offset)
                .and_then(|c| c.checked_add(span.text.as_str().len()))
                .ok_or(Error::Payload)?;
            *previous = Some(span.range.end());
        }
        Ok(())
    }
}
/// Validated source-sample recognition. An untimed segment remains text-only;
/// conversion to the captions owner refuses it instead of fabricating a duration.
#[derive(Clone, Debug)]
pub struct Recognition {
    language: LanguageTag,
    fingerprint: Fingerprint,
    sample_rate: u32,
    payload_fingerprint: NfcText,
    window: SampleRange,
    segments: Vec<RecognitionSegment>,
    alignment: Option<Fingerprint>,
}
impl Recognition {
    pub fn segments(&self) -> &[RecognitionSegment] {
        &self.segments
    }
    pub fn language(&self) -> &LanguageTag {
        &self.language
    }
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    pub fn window(&self) -> SampleRange {
        self.window
    }
    pub fn fingerprint(&self) -> &Fingerprint {
        &self.fingerprint
    }
    pub fn alignment_fingerprint(&self) -> Option<&Fingerprint> {
        self.alignment.as_ref()
    }
    pub fn is_text_only(&self) -> bool {
        self.segments.iter().any(|s| s.range.is_none())
    }
    /// Captions only has word storage today; character/token spans remain available
    /// via segments(). Unsupported units are not silently lost in conversion.
    pub fn to_transcript(&self) -> Result<Transcript, Error> {
        let mut segments = Vec::with_capacity(self.segments.len());
        for segment in &self.segments {
            let range = ticks(
                segment.range.ok_or(Error::TimingUnavailable)?,
                self.sample_rate,
            )?;
            let mut words = Vec::new();
            for span in &segment.timing {
                if span.unit != TimingUnit::Word {
                    return Err(Error::Unsupported);
                }
                words.push(Word {
                    text: span.text.clone(),
                    range: Some(ticks(span.range, self.sample_rate)?),
                    speaker: span.speaker.clone(),
                    confidence: span.confidence,
                });
            }
            segments.push(Segment {
                id: segment.id.clone(),
                text: segment.text.clone(),
                range,
                words,
            });
        }
        let mut fingerprint = self.fingerprint.clone();
        if let Some(alignment) = &self.alignment {
            let mut settings = String::from("transcribe+align-v1:");
            for value in [
                &fingerprint.settings,
                &alignment.provider,
                &alignment.model,
                &alignment.settings,
            ] {
                settings.push_str(&value.as_str().len().to_string());
                settings.push(':');
                settings.push_str(value.as_str());
            }
            fingerprint.settings = settings.into();
        }
        Ok(Transcript {
            source_lang: self.language.clone(),
            fingerprint,
            segments,
        })
    }
}
fn ticks(range: SampleRange, rate: u32) -> Result<SourceRange, Error> {
    let start = i64::try_from(range.start()).map_err(|_| Error::Invalid("sample overflow"))?;
    let end = i64::try_from(range.end()).map_err(|_| Error::Invalid("sample overflow"))?;
    // Floor both absolute boundaries once. No accumulated float durations/offsets.
    Ok(SourceRange::new(
        Tick::from_units(start, i64::from(rate), Rounding::Floor)?,
        Tick::from_units(end, i64::from(rate), Rounding::Floor)?,
    )?)
}
fn fingerprint(config: &crate::ProviderConfig) -> Fingerprint {
    Fingerprint {
        provider: config.id().clone(),
        model: config.model().clone(),
        settings: config.settings_fingerprint().clone(),
    }
}
/// Optional alignment is explicit: never select a different provider/model or
/// upload without its own request-scoped approval. No admissible aligner leaves
/// missing spans untimed. Real adapter/auth/quota failures propagate to engine.
pub struct Alignment<'a> {
    pub provider: &'a dyn Aligner,
    pub request: Request<'a>,
    pub unit: TimingUnit,
}

pub fn transcribe(
    provider: &dyn Transcriber,
    audio: AudioWindow<'_>,
    request: &Request<'_>,
    alignment: Option<Alignment<'_>>,
    hooks: &dyn ExecutionHooks,
) -> Result<Recognition, Error> {
    checkpoint(hooks, Stage::Transcribe)?;
    audio.validate(request.scope)?;
    if provider.config() != &request.scope.provider {
        return Err(Error::Unsupported);
    }
    if !request.scope.texts.is_empty() {
        return Err(Error::Invalid("transcription text payload"));
    }
    let caps = provider.capabilities(request.scope);
    request.prepare(
        &caps,
        &provider.estimate(request.scope),
        Operation::Transcribe,
    )?;
    let mut segments = provider.transcribe(audio, request, hooks)?;
    validate_ids(&segments)?;
    checkpoint(hooks, Stage::Assemble)?;
    for segment in &segments {
        segment.validate(audio.range)?;
        if segment.range.is_some() && !caps.timing.contains(&TimingUnit::Segment) {
            return Err(Error::Invalid("unadvertised segment timing"));
        }
        if segment
            .timing
            .iter()
            .any(|t| !caps.timing.contains(&t.unit) || t.provenance != Provenance::Native)
        {
            return Err(Error::Invalid("unadvertised native timing"));
        }
    }
    let mut alignment_fingerprint = None;
    if let Some(align) = alignment {
        let missing: Vec<_> = segments
            .iter()
            .filter(|s| {
                s.range.is_none()
                    || (align.unit != TimingUnit::Segment
                        && !s.timing.iter().any(|t| t.unit == align.unit))
            })
            .cloned()
            .collect();
        if !missing.is_empty() {
            checkpoint(hooks, Stage::Align)?;
            let scope = align.request.scope;
            if align.provider.config() != &scope.provider
                || scope.source_samples != audio.range
                || scope.sample_rate != audio.sample_rate
                || scope.payload_fingerprint != request.scope.payload_fingerprint
                || scope.source_language != request.scope.source_language
            {
                return Err(Error::Invalid("alignment request mismatch"));
            }
            let ac = align.provider.capabilities(scope);
            // Unsupported languages/timing are expected absence, not failed jobs.
            let admissible = ac.timing.contains(&align.unit)
                && match ac.check(scope) {
                    Ok(()) => true,
                    Err(Error::Unsupported | Error::UnsupportedLanguage) => false,
                    Err(error) => return Err(error),
                };
            if admissible {
                let expected: Vec<_> = missing.iter().map(|s| s.text.clone()).collect();
                if scope.texts != expected {
                    return Err(Error::ConsentRequired);
                }
                audio.validate(scope)?;
                align
                    .request
                    .prepare(&ac, &align.provider.estimate(scope), Operation::Align)?;
                let aligned = align
                    .provider
                    .align(audio, &missing, &align.request, hooks)?;
                checkpoint(hooks, Stage::Assemble)?;
                if aligned.len() != missing.len() {
                    return Err(Error::Invalid("alignment span count"));
                }
                for (original, aligned) in missing.iter().zip(aligned) {
                    aligned.validate(audio.range)?;
                    if original.id != aligned.id
                        || original.text != aligned.text
                        || aligned.range.is_none()
                        || (align.unit != TimingUnit::Segment
                            && !aligned.timing.iter().any(|t| t.unit == align.unit))
                        || aligned.timing.iter().any(|t| {
                            t.provenance != Provenance::Forced || !ac.timing.contains(&t.unit)
                        })
                    {
                        return Err(Error::Invalid("alignment changed text or omitted timing"));
                    }
                    let target = segments
                        .iter_mut()
                        .find(|s| s.id == original.id)
                        .ok_or(Error::Invalid("alignment identity"))?;
                    // Native segment ranges are authoritative; forced unit ranges
                    // must fit, rather than shifting the original speech interval.
                    let mut replacement = aligned;
                    replacement.range = original.range.or(replacement.range);
                    let native_units: BTreeSet<_> =
                        original.timing.iter().map(|s| s.unit).collect();
                    replacement
                        .timing
                        .retain(|s| !native_units.contains(&s.unit));
                    replacement.timing.extend(original.timing.clone());
                    replacement.validate(audio.range)?;
                    *target = replacement;
                }
                alignment_fingerprint = Some(fingerprint(align.provider.config()));
            }
        }
    }
    validate_ids(&segments)?;
    let result = Recognition {
        language: request.scope.source_language.clone(),
        fingerprint: fingerprint(provider.config()),
        sample_rate: audio.sample_rate,
        payload_fingerprint: request.scope.payload_fingerprint.clone(),
        window: audio.range,
        segments,
        alignment: alignment_fingerprint,
    };
    assemble(vec![result], hooks)
}
fn validate_ids(segments: &[RecognitionSegment]) -> Result<(), Error> {
    let mut ids = BTreeSet::new();
    if segments.iter().any(|s| !ids.insert(s.id.clone())) {
        return Err(Error::Invalid("duplicate segment identity"));
    }
    Ok(())
}
/// Merge chunk output in absolute sample coordinates. Only identical identified
/// observations (including ranges, timing layers and metadata) are deduplicated.
/// All other overlaps return a typed error, including equal-text repetitions.
/// Untimed chunks cannot be deduplicated by guessed positions.
pub fn assemble(
    chunks: Vec<Recognition>,
    hooks: &dyn ExecutionHooks,
) -> Result<Recognition, Error> {
    checkpoint(hooks, Stage::Assemble)?;
    let mut chunks = chunks.into_iter();
    let mut result = chunks
        .next()
        .ok_or(Error::Invalid("no recognition chunks"))?;
    let mut segments = std::mem::take(&mut result.segments);
    for chunk in chunks {
        checkpoint(hooks, Stage::Assemble)?;
        if chunk.payload_fingerprint != result.payload_fingerprint
            || chunk.language != result.language
            || chunk.sample_rate != result.sample_rate
            || chunk.fingerprint != result.fingerprint
        {
            return Err(Error::Invalid("incompatible recognition chunks"));
        }
        if let Some(alignment) = chunk.alignment {
            if result
                .alignment
                .as_ref()
                .is_some_and(|used| used != &alignment)
            {
                return Err(Error::Invalid("incompatible alignment fingerprints"));
            }
            result.alignment = Some(alignment);
        }
        result.window = SampleRange::new(
            result.window.start().min(chunk.window.start()),
            result.window.end().max(chunk.window.end()),
        )?;
        segments.extend(chunk.segments);
    }
    // Missing timing must preserve source chunk/text ordering; avoid placing
    // untimed speech at an invented source coordinate.
    if segments.iter().all(|s| s.range.is_some()) {
        segments.sort_by_key(|s| s.range.map(|r| r.start()));
        let mut merged: Vec<RecognitionSegment> = Vec::new();
        for segment in segments {
            if let Some(previous) = merged.last()
                && let (Some(a), Some(b)) = (previous.range, segment.range)
                && b.start() < a.end()
            {
                if *previous == segment {
                    continue;
                }
                return Err(Error::Invalid("ambiguous overlapping recognition"));
            }
            merged.push(segment);
        }
        segments = merged;
    }
    validate_ids(&segments)?;
    result.segments = segments;
    Ok(result)
}
