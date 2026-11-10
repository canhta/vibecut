use crate::contracts::checkpoint;
use crate::{
    Aligner, AudioWindow, Consent, Error, ExecutionHooks, Operation, Request, SampleRange, Scope,
    Stage, TimingUnit, Transcriber,
};
use captions::{
    Fingerprint, LanguageTag, NfcText, NormalizedText, Segment, SpanTiming, TimingSpans,
    Transcript, Word,
};
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
pub use captions::TimingProvenance as Provenance;
/// Provider-agnostic subword transport. Bytes can split UTF-8 scalars; decode
/// only their complete concatenation through captions' shared span primitive.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawToken {
    pub bytes: Vec<u8>,
    pub range: Option<SampleRange>,
    pub provenance: Provenance,
    pub confidence: Option<captions::Confidence>,
}
impl std::fmt::Debug for RawToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawToken")
            .field("range", &self.range)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecognitionSegment {
    pub id: NfcText,
    pub text: NfcText,
    pub range: Option<SampleRange>,
    pub timing: Vec<TimedText>,
    #[serde(default)]
    pub raw_tokens: Vec<RawToken>,
}
impl RecognitionSegment {
    fn has_timing(&self, unit: TimingUnit) -> bool {
        self.range.is_some()
            && (unit == TimingUnit::Segment
                || self.timing.iter().any(|t| t.unit == unit)
                || (unit == TimingUnit::Token
                    && !self.raw_tokens.is_empty()
                    && self.raw_tokens.iter().all(|t| t.range.is_some())))
    }

    fn validate(&self, window: SampleRange) -> Result<(), Error> {
        crate::contracts::policy::nonempty(&self.id)?;
        if self.range.is_some_and(|r| !window.contains(r)) {
            return Err(Error::Invalid("segment outside audio"));
        }
        if self.range.is_none()
            && (!self.timing.is_empty() || self.raw_tokens.iter().any(|t| t.range.is_some()))
        {
            return Err(Error::Invalid("unit timing without segment timing"));
        }
        if self
            .raw_tokens
            .len()
            .checked_mul(std::mem::size_of::<RawToken>())
            .is_none_or(|n| n > 16 * 1024 * 1024)
        {
            return Err(Error::Payload);
        }
        let mut previous = None;
        let mut size = 0usize;
        for token in &self.raw_tokens {
            size = size.checked_add(token.bytes.len()).ok_or(Error::Payload)?;
            if size > 16 * 1024 * 1024 || token.bytes.is_empty() {
                return Err(Error::Payload);
            }
            if token.range.is_some_and(|r| {
                !self.range.is_some_and(|s| s.contains(r))
                    || previous.is_some_and(|p| r.start() < p)
            }) {
                return Err(Error::Invalid("raw token timing"));
            }
            if let Some(range) = token.range {
                previous = Some(range.end());
            }
        }
        if !self.raw_tokens.is_empty() {
            if self.timing.iter().any(|s| s.unit == TimingUnit::Token) {
                return Err(Error::Invalid("duplicate token layer"));
            }
            let raw: Vec<u8> = self
                .raw_tokens
                .iter()
                .flat_map(|t| t.bytes.iter().copied())
                .collect();
            let raw = std::str::from_utf8(&raw).map_err(|_| Error::Invalid("raw token UTF-8"))?;
            if NormalizedText::new(raw)?.text() != &self.text {
                return Err(Error::Invalid("raw token text differs from recognition"));
            }
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
impl RecognitionSegment {
    fn caption_spans(&self, rate: u32) -> Result<Option<TimingSpans>, Error> {
        if self.raw_tokens.is_empty() && self.timing.iter().all(|s| s.unit == TimingUnit::Word) {
            return Ok(None);
        }
        let bytes: Vec<u8> = self
            .raw_tokens
            .iter()
            .flat_map(|t| t.bytes.iter().copied())
            .collect();
        let raw = if self.raw_tokens.is_empty() {
            self.text.as_str()
        } else {
            std::str::from_utf8(&bytes).map_err(|_| Error::Invalid("raw token UTF-8"))?
        };
        let normalized = NormalizedText::new(raw)?;
        let mut inputs = Vec::new();
        let mut offset = 0;
        for (index, token) in self.raw_tokens.iter().enumerate() {
            let end = offset + token.bytes.len();
            inputs.push(SpanTiming {
                index,
                unit: TimingUnit::Token,
                raw: offset..end,
                range: token.range.map(|r| ticks(r, rate)).transpose()?,
                provenance: token.provenance,
                speaker: None,
                confidence: token.confidence,
            });
            offset = end;
        }
        let mut cursors = BTreeMap::new();
        for span in &self.timing {
            let (cursor, index) = cursors.entry(span.unit).or_insert((0, 0));
            let remaining = self
                .text
                .as_str()
                .get(*cursor..)
                .ok_or(Error::Invalid("text boundary"))?;
            let start = *cursor
                + remaining
                    .find(span.text.as_str())
                    .ok_or(Error::Invalid("unit text differs from recognition"))?;
            let end = start + span.text.as_str().len();
            let raw = normalized
                .raw_range(start..end)
                .ok_or(Error::Invalid("unit text boundary"))?;
            inputs.push(SpanTiming {
                index: *index,
                unit: span.unit,
                raw,
                range: Some(ticks(span.range, rate)?),
                provenance: span.provenance,
                speaker: span.speaker.clone(),
                confidence: span.confidence,
            });
            *cursor = end;
            *index += 1;
        }
        Ok(Some(TimingSpans::new(raw, inputs, None)?))
    }
}
/// Validated source-sample recognition. An untimed segment remains text-only;
/// conversion to the captions owner refuses it instead of fabricating a duration.
/// Serialization is the engine checkpoint form; cache integrity is checked before use.
#[derive(Clone, Debug, Serialize, Deserialize)]
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
    /// Whether any retained segment lacks the requested timing layer.
    pub fn needs_timing(&self, unit: TimingUnit) -> bool {
        self.segments
            .iter()
            .any(|segment| !segment.has_timing(unit))
    }
    pub fn is_text_only(&self) -> bool {
        self.segments.iter().any(|s| s.range.is_none())
    }
    /// Retain source unit identity, optional raw-token timing and raw/NFC span
    /// correspondence. Source samples floor once into ticks; tokens remain tokens.
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
                    continue;
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
                spans: segment.caption_spans(self.sample_rate)?,
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
    let segments = provider.transcribe(audio, request, hooks)?;
    validate_ids(&segments)?;
    checkpoint(hooks, Stage::Assemble)?;
    for segment in &segments {
        segment.validate(audio.range)?;
        if segment.range.is_some() && !caps.timing.contains(&TimingUnit::Segment) {
            return Err(Error::Invalid("unadvertised segment timing"));
        }
        if segment.raw_tokens.iter().any(|t| {
            (t.range.is_some() && !caps.timing.contains(&TimingUnit::Token))
                || t.provenance != Provenance::Native
        }) {
            return Err(Error::Invalid("unadvertised native tokens"));
        }
        if segment
            .timing
            .iter()
            .any(|t| !caps.timing.contains(&t.unit) || t.provenance != Provenance::Native)
        {
            return Err(Error::Invalid("unadvertised native timing"));
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
        alignment: None,
    };
    let result = match alignment {
        Some(alignment) => align(
            result,
            alignment.provider,
            audio,
            alignment.request.scope,
            alignment.request.consent,
            alignment.unit,
            hooks,
        )?,
        None => result,
    };
    assemble(vec![result], hooks)
}

/// Apply one explicitly selected aligner to an already validated recognition.
/// A local aligner may receive a template scope with empty texts; this function
/// binds the exact missing segment texts before validation and execution. A remote
/// aligner must already have its text-bound scope/consent. Existing provider,
/// model, source fingerprint, sample coordinates and native timing provenance are
/// retained; no timing is fabricated when the aligner is inadmissible.
#[allow(clippy::too_many_arguments)]
pub fn align(
    mut recognition: Recognition,
    provider: &dyn Aligner,
    audio: AudioWindow<'_>,
    scope: &Scope,
    consent: Option<&Consent>,
    unit: TimingUnit,
    hooks: &dyn ExecutionHooks,
) -> Result<Recognition, Error> {
    checkpoint(hooks, Stage::Align)?;
    if provider.config() != &scope.provider
        || scope.operation != Operation::Align
        || recognition.window != audio.range
        || recognition.sample_rate != audio.sample_rate
        || scope.source_samples != audio.range
        || scope.sample_rate != audio.sample_rate
        || scope.payload_fingerprint != recognition.payload_fingerprint
        || scope.source_language != recognition.language
    {
        return Err(Error::Invalid("alignment request mismatch"));
    }
    let missing: Vec<_> = recognition
        .segments
        .iter()
        .filter(|segment| !segment.has_timing(unit))
        .cloned()
        .collect();
    if missing.is_empty() {
        return Ok(recognition);
    }
    let capabilities = provider.capabilities(scope);
    let admissible = capabilities.timing.contains(&unit)
        && match capabilities.check(scope) {
            Ok(()) => true,
            Err(Error::Unsupported | Error::UnsupportedLanguage) => false,
            Err(error) => return Err(error),
        };
    if !admissible {
        return Ok(recognition);
    }
    let expected: Vec<_> = missing.iter().map(|segment| segment.text.clone()).collect();
    let bound;
    let bound_scope = if scope.texts.is_empty() {
        if provider.config().is_remote() {
            return Err(Error::Invalid(
                "remote alignment requires a text-bound scope",
            ));
        }
        bound = Scope {
            texts: expected.clone(),
            ..scope.clone()
        };
        &bound
    } else {
        if scope.texts != expected {
            return Err(Error::ConsentRequired);
        }
        scope
    };
    audio.validate(bound_scope)?;
    let capabilities = provider.capabilities(bound_scope);
    let request = Request {
        scope: bound_scope,
        consent,
    };
    request.prepare(
        &capabilities,
        &provider.estimate(bound_scope),
        Operation::Align,
    )?;
    let aligned = provider.align(audio, &missing, &request, hooks)?;
    checkpoint(hooks, Stage::Assemble)?;
    if aligned.len() != missing.len() {
        return Err(Error::Invalid("alignment span count"));
    }
    for (original, aligned) in missing.iter().zip(aligned) {
        aligned.validate(audio.range)?;
        if original.id != aligned.id
            || original.text != aligned.text
            || !aligned.has_timing(unit)
            || aligned.raw_tokens.iter().any(|token| {
                token.provenance != Provenance::Forced
                    || !capabilities.timing.contains(&TimingUnit::Token)
            })
            || aligned.timing.iter().any(|timing| {
                timing.provenance != Provenance::Forced
                    || !capabilities.timing.contains(&timing.unit)
            })
        {
            return Err(Error::Invalid("alignment changed text or omitted timing"));
        }
        let target = recognition
            .segments
            .iter_mut()
            .find(|segment| segment.id == original.id)
            .ok_or(Error::Invalid("alignment identity"))?;
        let mut replacement = aligned;
        replacement.range = original.range.or(replacement.range);
        let native_units: BTreeSet<_> = original.timing.iter().map(|timing| timing.unit).collect();
        replacement
            .timing
            .retain(|timing| !native_units.contains(&timing.unit));
        replacement.timing.extend(original.timing.clone());
        if !original.raw_tokens.is_empty() {
            let forced_tokens = &replacement.raw_tokens;
            let fill_missing = original
                .raw_tokens
                .iter()
                .any(|token| token.range.is_none())
                && !forced_tokens.is_empty();
            if fill_missing
                && (original.raw_tokens.len() != forced_tokens.len()
                    || original
                        .raw_tokens
                        .iter()
                        .zip(forced_tokens)
                        .any(|(native, forced)| native.bytes != forced.bytes))
            {
                return Err(Error::Invalid("alignment changed token identity"));
            }
            let mut tokens = original.raw_tokens.clone();
            if fill_missing {
                for (native, forced) in tokens.iter_mut().zip(forced_tokens) {
                    if native.range.is_none() && forced.range.is_some() {
                        native.range = forced.range;
                        native.provenance = Provenance::Forced;
                    }
                }
            }
            replacement.raw_tokens = tokens;
        }
        if native_units.contains(&TimingUnit::Token) {
            replacement.raw_tokens.clear();
        } else if !replacement.raw_tokens.is_empty() {
            replacement
                .timing
                .retain(|timing| timing.unit != TimingUnit::Token);
        }
        replacement.validate(audio.range)?;
        if !replacement.has_timing(unit) {
            return Err(Error::Invalid("alignment omitted requested timing"));
        }
        *target = replacement;
    }
    recognition.alignment = Some(fingerprint(provider.config()));
    Ok(recognition)
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
