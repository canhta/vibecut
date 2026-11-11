//! Transcribes the speech windows of one source into source-timed cues.
use super::checkpoints::{hex, transcribe_window};
use super::error::Error as TranscriptError;
use super::operation::{Cancellation, Progress, Stage, Transcription};
use crate::Error as EngineError;
use crate::cache::ArtifactCache;
use crate::providers::{ConsentPrompt, Cost, Error as ProviderError, Providers};
use ai::SampleRange;
use captions::LanguageTag;
use captions::{Cue, NfcText, SegmentRules, SegmentedCue, Transcript};
use media::{SourceAsset, SpeechExtractSettings, SpeechWindow};
use std::{borrow::BorrowMut, sync::Mutex};
use std::{path::Path, time::Instant};
use time::{FrameRate, Rounding, SourceRange, Tick};
pub(crate) const MIN_CAPTION_SECONDS: i64 = 30;
pub(crate) const MAX_CAPTION_SECONDS: i64 = 300;
/// Working-memory ceiling for synchronous speech extraction, ASR and alignment.
/// Blocking composition roots reserve this from their process budget before execution;
/// a workflow job declares it in its JobSpec instead.
pub const WORK_BYTES: u64 = 64 * 1024 * 1024;
/// Recognition/result ceiling retained alongside synchronous ASR working memory.
/// A composition already holding a larger result lease must not charge it twice.
pub const RESULT_BYTES: u64 = 16 * 1024 * 1024;
/// Below this mean word confidence the run is reported as a likely wrong
/// language instead of inserting cues a creator would have to delete.
const MIN_MEAN_CONFIDENCE: f64 = 0.5;

/// Run the shared ASR-to-cue stage. The caller has already imported and validated
/// media and resolved the source language. Range is in source ticks (None means full).
/// The cache may be owned or borrowed behind its mutex; no lock spans provider IO.
/// Progress and execution hooks observe the same operation cancellation flag.
/// The composition root must already hold WORK_BYTES plus RESULT_BYTES in the
/// shared ByteBudget (or its workflow job's equivalent work/result reservations).
/// Invalid ranges/languages, provider failures, missing speech, low confidence and
/// cancellation return typed errors without partial cues; completed raw checkpoints remain.
#[allow(clippy::too_many_arguments)]
pub fn transcribe<C: BorrowMut<ArtifactCache>>(
    asset: &SourceAsset,
    duration: Tick,
    source_language: &LanguageTag,
    providers: &Providers,
    range: Option<SourceRange>,
    cache: &Mutex<C>,
    consent: &dyn ConsentPrompt,
    progress: &dyn Progress,
    cancel: &Cancellation,
    hooks: &dyn ai::ExecutionHooks,
) -> Result<Transcription, EngineError> {
    if !["vi", "en", "zh"]
        .iter()
        .any(|tag| *tag == source_language.as_str())
    {
        return Err(EngineError::Edit(crate::EditError::Unsupported(
            "source language must be vi, en or zh",
        )));
    }
    cancel.checkpoint()?;
    let started = Instant::now();
    let profile = providers.language_profile(source_language)?;
    let speech_settings = SpeechExtractSettings {
        // Text-only providers are timed by the local 30 s DTW aligner.
        max_duration: Tick::from_seconds_f64(
            if providers.needs_alignment {
                MIN_CAPTION_SECONDS as f64
            } else {
                MAX_CAPTION_SECONDS as f64
            },
            Rounding::Floor,
        )?,
        overlap: Tick::ZERO,
    };
    let full_range = range.unwrap_or(SourceRange::new(Tick::ZERO, duration)?);
    if full_range.start() < Tick::ZERO || full_range.end() > duration {
        return Err(EngineError::Edit(crate::EditError::Invalid(
            "transcription range outside asset",
        )));
    }
    let fingerprint = NfcText::from(hex(&asset.content_hash().0));
    let mut recognitions = Vec::new();
    let mut checkpoints = 0usize;
    let mut cost = Cost::Free;
    let mut failure = None;
    let mut recognition_bytes = 0usize;
    cancel.checkpoint()?;
    progress.stage(Stage::Extract);
    let extraction = media::extract_speech_windows_with_cancel(
        asset,
        full_range,
        &speech_settings,
        cancel.media_flag(),
        |window| {
            if cancel.cancelled() || hooks.cancelled() {
                return Err(media::MediaError::Cancelled);
            }
            if !window.speech_detected {
                return Ok(());
            }
            progress.stage(Stage::Transcribe);
            match transcribe_window(
                providers,
                cache,
                consent,
                progress,
                hooks,
                source_language,
                &fingerprint,
                window,
                &mut cost,
                &mut checkpoints,
            ) {
                Ok(recognition) => {
                    recognition_bytes = recognition_bytes.saturating_add(
                        crate::session::wire::bounded_json(&recognition, 16 << 20)
                            .map_or(usize::MAX, |bytes| bytes.len()),
                    );
                    if recognition_bytes > 16 << 20 {
                        failure = Some(EngineError::Control(
                            crate::session::wire::ControlError::Limit,
                        ));
                        return Err(media::MediaError::Cancelled);
                    }
                    recognitions.push(recognition);
                    Ok(())
                }
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
    match extraction {
        Ok(()) => {}
        Err(media::MediaError::Cancelled) => {
            return Err(EngineError::Runtime(crate::RuntimeError::Cancelled));
        }
        Err(error) => return Err(error.into()),
    }
    cancel.checkpoint()?;
    if recognitions.is_empty() {
        return Err(TranscriptError::NoSpeech.into());
    }

    cancel.checkpoint()?;
    progress.stage(Stage::Assemble);
    let recognition =
        ai::assemble(recognitions, hooks).map_err(super::checkpoints::provider_error)?;
    let transcript = recognition.to_transcript()?;
    if mean_confidence(&transcript).is_some_and(|mean| mean < MIN_MEAN_CONFIDENCE) {
        return Err(TranscriptError::LowConfidence.into());
    }
    cancel.checkpoint()?;
    progress.stage(Stage::Segment);
    let rules = SegmentRules::from_profile(&profile, FrameRate::FPS_30)?;
    let segmentation = captions::segment_transcript(&transcript, &profile, &rules)?;
    let cues: Vec<Cue> = segmentation
        .cues
        .into_iter()
        .map(|SegmentedCue { cue, .. }| cue)
        .collect();
    if cues.is_empty() {
        return Err(TranscriptError::LowConfidence.into());
    }
    Ok(Transcription {
        source_language: source_language.clone(),
        transcript,
        cues,
        cost,
        checkpoints,
        elapsed: started.elapsed(),
    })
}

impl Providers {
    /// Import a media path and detect the spoken language without opening a
    /// translator. Composition roots use this to decide whether translation (and
    /// its credentials) is required at all.
    pub fn detect_language_for(
        &self,
        input: &Path,
        cancel: &Cancellation,
    ) -> Result<LanguageTag, EngineError> {
        cancel.checkpoint()?;
        let asset = SourceAsset::import(media::AssetId(1), input.to_path_buf())?;
        let duration = media_duration(&asset)?;
        self.detect_language(&asset, duration, cancel)
    }

    /// Auto-detect the spoken language of an already-imported asset. The caller
    /// then supplies the returned tag explicitly to [`transcribe`].
    pub fn detect_language(
        &self,
        asset: &SourceAsset,
        duration: Tick,
        cancel: &Cancellation,
    ) -> Result<LanguageTag, EngineError> {
        self.detect_language_with_hooks(asset, duration, cancel, cancel)
    }

    pub(crate) fn detect_language_with_hooks(
        &self,
        asset: &SourceAsset,
        duration: Tick,
        cancel: &Cancellation,
        hooks: &dyn ai::ExecutionHooks,
    ) -> Result<LanguageTag, EngineError> {
        cancel.checkpoint()?;
        let end = duration.min(Tick::from_seconds_f64(
            MIN_CAPTION_SECONDS as f64,
            Rounding::Floor,
        )?);
        let range = SourceRange::new(Tick::ZERO, end)?;
        let settings = SpeechExtractSettings {
            max_duration: end,
            overlap: Tick::ZERO,
        };
        let mut detected = None;
        let mut failure = None;
        let result = media::extract_speech_windows_with_cancel(
            asset,
            range,
            &settings,
            cancel.media_flag(),
            |window| {
                if cancel.cancelled() {
                    return Err(media::MediaError::Cancelled);
                }
                match detection_window(&window).and_then(|(samples, range)| {
                    self.transcriber
                        .detect_language(
                            ai::AudioWindow {
                                samples,
                                range,
                                sample_rate: media::SPEECH_SAMPLE_RATE,
                            },
                            hooks,
                        )
                        .map_err(|error| match error {
                            ai::Error::Unsupported => ProviderError::AutoLanguageUnsupported.into(),
                            ai::Error::Cancelled => {
                                EngineError::Runtime(crate::RuntimeError::Cancelled)
                            }
                            error => EngineError::Ai(error),
                        })
                }) {
                    Ok(language) => {
                        detected = Some(language);
                        Ok(())
                    }
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
        match result {
            Ok(()) => {}
            Err(media::MediaError::Cancelled) => {
                return Err(EngineError::Runtime(crate::RuntimeError::Cancelled));
            }
            Err(error) => return Err(error.into()),
        }
        cancel.checkpoint()?;
        Ok(detected.ok_or(TranscriptError::NoSpeech)?)
    }
}

/// Mean confidence over recognized words that carry a confidence. Providers
/// without confidence data return `None` and are never labelled low confidence.
fn mean_confidence(transcript: &Transcript) -> Option<f64> {
    let (sum, count) = transcript
        .segments
        .iter()
        .flat_map(|segment| &segment.words)
        .filter_map(|word| word.confidence)
        .fold((0.0, 0usize), |(sum, count), confidence| {
            (sum + f64::from(f32::from(confidence)), count + 1)
        });
    (count > 0).then(|| sum / count as f64)
}

fn detection_window(window: &SpeechWindow) -> Result<(&[f32], SampleRange), EngineError> {
    let (start, end) = sample_range(window)?;
    Ok((window.samples.as_slice(), SampleRange::new(start, end)?))
}

pub(crate) fn media_duration(asset: &SourceAsset) -> Result<Tick, EngineError> {
    asset
        .probe()
        .duration
        .filter(|value| *value > Tick::ZERO)
        .ok_or(EngineError::Edit(crate::EditError::Unsupported(
            "media duration is unknown",
        )))
}

pub(crate) fn sample_range(window: &SpeechWindow) -> Result<(u64, u64), EngineError> {
    let start = window
        .source_range
        .start()
        .to_units(i64::from(media::SPEECH_SAMPLE_RATE), Rounding::Nearest)?;
    let start = u64::try_from(start)
        .map_err(|_| EngineError::Edit(crate::EditError::Invalid("sample origin")))?;
    let size = u64::try_from(window.samples.len())
        .map_err(|_| EngineError::Control(crate::session::wire::ControlError::Limit))?;
    let end = start.checked_add(size).ok_or(EngineError::Control(
        crate::session::wire::ControlError::Limit,
    ))?;
    Ok((start, end))
}
