//! Opt-in ElevenLabs Scribe preparation of a real transcript.
//!
//! This is upstream of the clip pipeline: it decodes bounded speech windows,
//! transcribes each with the already-admitted Scribe adapter, assembles absolute
//! source coordinates and caches the resulting captions transcript.
use crate::budget::Budget;
use crate::{Error, Result};
use ai::elevenlabs::{ElevenLabsTranscriber, MODEL};
use ai::transport::UreqTransport;
use ai::{
    AudioWindow, Consent, CostEstimate, ExecutionHooks, Money, Request, SampleRange, Scope, Stage,
    Transcriber,
};
use captions::{
    LanguageProfile, LanguageRegistry, LanguageTag, NfcText, TimingSpans, Transcript, Word,
};
use engine::providers::Credentials;
use media::{AssetId, SourceAsset, SpeechExtractSettings, extract_speech_windows_with_cancel};
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Instant;
use time::{Rounding, SourceRange, TICKS_PER_SECOND, Tick};

const SAMPLE_RATE: u32 = 16_000;
const MAX_PCM_BYTES: u64 = 64 * 1024 * 1024;
const CHUNK_SECONDS: i64 = 15 * 60;

struct NoHooks;
impl ExecutionHooks for NoHooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn stage(&self, _: Stage) {}
}

pub struct Prepared {
    pub transcript: Transcript,
    pub cost_micros: u64,
    pub wall_ms: u64,
    pub chunks: usize,
    pub notes: Vec<String>,
}

/// Transcribe a source video into a captions transcript. Caller supplies the
/// exact language, source content hash and cost cap; this never retries or falls
/// back to another provider.
pub fn prepare(
    source: &Path,
    language: &LanguageTag,
    source_sha: &str,
    budget: &mut Budget,
) -> Result<Prepared> {
    let started = Instant::now();
    let key = Credentials::from_env().elevenlabs()?;
    let config = ElevenLabsTranscriber::provider_config(MODEL)?;
    let transcriber = ElevenLabsTranscriber::new(config, key, Arc::new(UreqTransport::new()))?;
    let asset = SourceAsset::import(AssetId(1), source.to_path_buf())?;
    let duration = asset
        .probe()
        .duration
        .ok_or_else(|| Error::Options("source duration is unknown".into()))?;
    let range = SourceRange::new(Tick::ZERO, duration)?;
    let settings = SpeechExtractSettings {
        max_duration: Tick(CHUNK_SECONDS * TICKS_PER_SECOND),
        overlap: Tick::ZERO,
    };
    let mut recognitions = Vec::new();
    let mut cost_micros = 0u64;
    let mut notes = Vec::new();
    let mut failure = None;
    let mut windows = 0usize;
    let extraction = extract_speech_windows_with_cancel(
        &asset,
        range,
        &settings,
        Arc::new(AtomicBool::new(false)),
        |window| {
            windows += 1;
            let prepared = (|| -> Result<()> {
                let expected = expected_samples(window.source_range)?;
                let samples = fit_samples(window.samples, expected, &mut notes);
                let start = samples_from_tick(window.source_range.start())?;
                let end = start
                    .checked_add(u64::try_from(samples.len()).map_err(|_| Error::Payload)?)
                    .ok_or(Error::Payload)?;
                let sample_range = SampleRange::new(start, end)?;
                if sample_range.len().saturating_mul(4) > MAX_PCM_BYTES {
                    return Err(Error::Payload);
                }
                let scope = Scope::transcription(
                    transcriber.config().clone(),
                    ai::elevenlabs::MODE.into(),
                    language.clone(),
                    sample_range,
                    SAMPLE_RATE,
                    format!("asr:{source_sha}").into(),
                )?;
                let estimate = estimate_micros(&transcriber.estimate(&scope))?;
                budget.reserve(estimate)?;
                cost_micros = cost_micros.saturating_add(estimate);
                let consent = Consent::grant(scope.clone(), Money::new("USD", estimate)?)?;
                let request = Request {
                    scope: &scope,
                    consent: Some(&consent),
                };
                let audio = AudioWindow {
                    samples: &samples,
                    range: sample_range,
                    sample_rate: SAMPLE_RATE,
                };
                recognitions.push(ai::transcribe(
                    &transcriber,
                    audio,
                    &request,
                    None,
                    &NoHooks,
                )?);
                Ok(())
            })();
            match prepared {
                Ok(()) => Ok(()),
                Err(error) => {
                    failure = Some(error);
                    Err(media::MediaError::Cancelled)
                }
            }
        },
    );
    if let Some(error) = failure {
        return Err(error);
    }
    extraction?;
    let recognition = ai::assemble(recognitions, &NoHooks)?;
    let mut transcript = recognition.to_transcript()?;
    let registry = LanguageRegistry::standard()?;
    let profile = registry.profile(language);
    for segment in &mut transcript.segments {
        if let Some(spans) = &segment.spans {
            match span_words(spans, profile) {
                Ok(Some(words)) => {
                    segment.words = words;
                    segment.spans = None;
                }
                Ok(None) => notes.push("ASR spans could not be reduced to timed words".into()),
                Err(error) => notes.push(format!("ASR span grouping failed: {error}")),
            }
        }
    }
    Ok(Prepared {
        transcript,
        cost_micros,
        wall_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        chunks: windows,
        notes,
    })
}

fn span_words(spans: &TimingSpans, profile: &LanguageProfile) -> Result<Option<Vec<Word>>> {
    let display = spans.display().as_str();
    let groups = spans.groups(profile)?;
    let mut merged: Vec<(usize, usize, time::SourceRange, Option<NfcText>)> = Vec::new();
    for group in groups {
        let slice = display
            .get(group.display.clone())
            .ok_or(captions::Error::Invalid("timing group text"))?;
        if slice.trim().is_empty() {
            continue;
        }
        match group.range {
            Some(range) => {
                let leading = slice.len() - slice.trim_start().len();
                let trailing = slice.len() - slice.trim_end().len();
                let start = group.display.start.saturating_add(leading);
                let end = group.display.end.saturating_sub(trailing);
                if start >= end {
                    continue;
                }
                merged.push((start, end, range, group.speaker));
            }
            None => {
                if let Some(previous) = merged.last_mut() {
                    previous.1 = group.display.end;
                }
            }
        }
    }
    if merged.is_empty() {
        return Ok(None);
    }
    let mut words = Vec::with_capacity(merged.len());
    for (start, end, range, speaker) in merged {
        let text = display
            .get(start..end)
            .ok_or(captions::Error::Invalid("timing word text"))?;
        if text.trim().is_empty() {
            continue;
        }
        words.push(Word {
            text: NfcText::from(text),
            range: Some(range),
            speaker,
            confidence: None,
        });
    }
    if words.is_empty() {
        Ok(None)
    } else {
        Ok(Some(words))
    }
}

fn expected_samples(range: SourceRange) -> Result<usize> {
    let count = range
        .duration()
        .to_units(i64::from(SAMPLE_RATE), Rounding::Nearest)?;
    usize::try_from(count).map_err(|_| Error::Payload)
}

fn fit_samples(mut samples: Vec<f32>, expected: usize, notes: &mut Vec<String>) -> Vec<f32> {
    if samples.len() > expected {
        notes.push(format!(
            "ASR chunk trimmed {} trailing samples",
            samples.len() - expected
        ));
        samples.truncate(expected);
    } else if samples.len() < expected {
        notes.push(format!(
            "ASR chunk padded {} missing samples",
            expected - samples.len()
        ));
        samples.resize(expected, 0.0);
    }
    samples
}

fn samples_from_tick(tick: Tick) -> Result<u64> {
    let samples = tick.to_units(i64::from(SAMPLE_RATE), Rounding::Nearest)?;
    u64::try_from(samples).map_err(|_| Error::Payload)
}

fn estimate_micros(estimate: &CostEstimate) -> Result<u64> {
    match estimate {
        CostEstimate::Free => Ok(0),
        CostEstimate::UpperBound(money) => Ok(money.micros()),
        CostEstimate::Unknown => Err(Error::UnknownCost),
    }
}

/// Cache path for a source whose transcript was prepared by Scribe.
pub fn cache_path(out_dir: &Path, source_sha: &str) -> PathBuf {
    out_dir
        .join("cache/transcripts")
        .join(format!("{source_sha}-elevenlabs-{MODEL}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_fit_is_exact_and_records_repairs() {
        let mut notes = Vec::new();
        assert_eq!(fit_samples(vec![0.0; 3], 5, &mut notes), vec![0.0; 5]);
        assert_eq!(notes.len(), 1);
        let mut notes = Vec::new();
        assert_eq!(fit_samples(vec![0.0; 7], 5, &mut notes), vec![0.0; 5]);
        assert_eq!(notes.len(), 1);
        let mut notes = Vec::new();
        assert_eq!(fit_samples(vec![0.0; 5], 5, &mut notes), vec![0.0; 5]);
        assert!(notes.is_empty());
    }

    #[test]
    fn untimed_punctuation_attaches_to_the_previous_word() {
        use captions::{SpanTiming, TimingProvenance, TimingUnit};
        let range = SourceRange::new(Tick::ZERO, Tick(TICKS_PER_SECOND)).unwrap();
        let spans = TimingSpans::new(
            "Hello!",
            vec![
                SpanTiming {
                    index: 0,
                    unit: TimingUnit::Token,
                    raw: 0..5,
                    range: Some(range),
                    provenance: TimingProvenance::Native,
                    speaker: None,
                    confidence: None,
                },
                SpanTiming {
                    index: 1,
                    unit: TimingUnit::Token,
                    raw: 5..6,
                    range: None,
                    provenance: TimingProvenance::Native,
                    speaker: None,
                    confidence: None,
                },
            ],
            None,
        )
        .unwrap();
        let registry = LanguageRegistry::standard().unwrap();
        let profile = registry.profile(&LanguageTag::parse("en").unwrap());
        let words = span_words(&spans, profile).unwrap().unwrap();
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].text.as_str(), "Hello!");
        assert_eq!(words[0].range, Some(range));
    }
}
