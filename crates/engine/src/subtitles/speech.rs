//! Acquires source-timed speech recognition.
use super::{
    Arc, ArtifactCache, AtomicBool, Cancellation, ConsentPrompt, Cost, Cue, EditError, FrameRate,
    IdleHooks, Instant, JobRunner, LanguageTag, MAX_CAPTION_SECONDS, MIN_CAPTION_SECONDS, NfcText,
    Path, Progress, Providers, Rounding, SampleRange, SegmentRules, SegmentedCue, SilentProgress,
    SourceAsset, SourceRange, SpeechExtractSettings, SpeechWindow, Stage, Tick, Transcription,
    WORK_BYTES, hex, transcribe_window,
};
impl Cancellation {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.flag.store(true, std::sync::atomic::Ordering::Release);
    }
    pub fn cancelled(&self) -> bool {
        self.flag.load(std::sync::atomic::Ordering::Acquire)
    }
    pub(crate) fn media_flag(&self) -> Arc<AtomicBool> {
        self.flag.clone()
    }
    pub(crate) fn checkpoint(&self) -> Result<(), EditError> {
        if self.cancelled() {
            Err(EditError::Cancelled)
        } else {
            Ok(())
        }
    }
}
impl Progress for SilentProgress {
    fn stage(&self, _: Stage) {}
}

/// Run the shared ASR-to-cue stage. The caller has already imported and validated
/// media and resolved the source language; jobs, cache and consent stay caller-owned.
#[allow(clippy::too_many_arguments)]
pub fn transcribe(
    asset: &SourceAsset,
    duration: Tick,
    source_language: &LanguageTag,
    providers: &Providers,
    runner: &mut JobRunner,
    cache: &mut ArtifactCache,
    consent: &dyn ConsentPrompt,
    progress: &dyn Progress,
    cancel: &Cancellation,
) -> Result<Transcription, EditError> {
    if !["vi", "en", "zh"]
        .iter()
        .any(|tag| *tag == source_language.as_str())
    {
        return Err(EditError::Unsupported(
            "source language must be vi, en or zh",
        ));
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
        max_bytes: WORK_BYTES,
        bit_rate: 48_000,
    };
    let full_range = SourceRange::new(Tick::ZERO, duration)?;
    let fingerprint = NfcText::from(hex(&asset.content_hash().0));
    let mut recognitions = Vec::new();
    let mut checkpoints = 0usize;
    let mut cost = Cost::Free;
    let mut failure = None;
    cancel.checkpoint()?;
    progress.stage(Stage::Extract);
    let extraction = media::extract_speech_windows_with_cancel(
        asset,
        full_range,
        &speech_settings,
        cancel.media_flag(),
        |window| {
            if cancel.cancelled() {
                return Err(media::MediaError::Cancelled);
            }
            if !window.speech_detected {
                return Ok(());
            }
            progress.stage(Stage::Transcribe);
            match transcribe_window(
                providers,
                runner,
                cache,
                consent,
                progress,
                cancel,
                source_language,
                &fingerprint,
                window,
                &mut cost,
                &mut checkpoints,
            ) {
                Ok(recognition) => {
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
        Err(media::MediaError::Cancelled) => return Err(EditError::Cancelled),
        Err(error) => return Err(error.into()),
    }
    cancel.checkpoint()?;
    if recognitions.is_empty() {
        return Err(EditError::Unsupported("no speech detected"));
    }

    cancel.checkpoint()?;
    progress.stage(Stage::Assemble);
    let recognition = ai::assemble(recognitions, &IdleHooks)?;
    let transcript = recognition.to_transcript()?;
    cancel.checkpoint()?;
    progress.stage(Stage::Segment);
    let rules = SegmentRules::from_profile(&profile, FrameRate::FPS_30)?;
    let segmentation = captions::segment_transcript(
        &transcript,
        &profile,
        &rules,
        &captions::LayoutHooks::default(),
    )?;
    let cues: Vec<Cue> = segmentation
        .cues
        .into_iter()
        .map(|SegmentedCue { cue, .. }| cue)
        .collect();
    if cues.is_empty() {
        return Err(EditError::Unsupported("no subtitle cues generated"));
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
    ) -> Result<LanguageTag, EditError> {
        cancel.checkpoint()?;
        let asset = SourceAsset::import(media::AssetId(1), input.to_path_buf())?;
        let duration = media_duration(&asset)?;
        self.detect_language(&asset, duration, cancel)
    }

    pub(super) fn detect_language(
        &self,
        asset: &SourceAsset,
        duration: Tick,
        cancel: &Cancellation,
    ) -> Result<LanguageTag, EditError> {
        cancel.checkpoint()?;
        let whisper = self
            .transcriber
            .whisper()
            .ok_or(EditError::AutoLanguageUnsupported)?;
        let end = duration.min(Tick::from_seconds_f64(
            MIN_CAPTION_SECONDS as f64,
            Rounding::Floor,
        )?);
        let range = SourceRange::new(Tick::ZERO, end)?;
        let settings = SpeechExtractSettings {
            max_duration: end,
            overlap: Tick::ZERO,
            max_bytes: WORK_BYTES,
            bit_rate: 48_000,
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
                    whisper
                        .detect_language(
                            ai::AudioWindow {
                                samples,
                                range,
                                sample_rate: media::SPEECH_SAMPLE_RATE,
                            },
                            &IdleHooks,
                        )
                        .map_err(EditError::Ai)
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
            Err(media::MediaError::Cancelled) => return Err(EditError::Cancelled),
            Err(error) => return Err(error.into()),
        }
        cancel.checkpoint()?;
        detected.ok_or(EditError::Unsupported(
            "no speech detected for language detection",
        ))
    }
}

fn detection_window(window: &SpeechWindow) -> Result<(&[f32], SampleRange), EditError> {
    let (start, end) = sample_range(window)?;
    Ok((window.samples.as_slice(), SampleRange::new(start, end)?))
}

pub(super) fn media_duration(asset: &SourceAsset) -> Result<Tick, EditError> {
    asset
        .probe()
        .duration
        .filter(|value| *value > Tick::ZERO)
        .ok_or(EditError::Unsupported("media duration is unknown"))
}

pub(super) fn sample_range(window: &SpeechWindow) -> Result<(u64, u64), EditError> {
    let start = window
        .source_range
        .start()
        .to_units(i64::from(media::SPEECH_SAMPLE_RATE), Rounding::Nearest)?;
    let start = u64::try_from(start).map_err(|_| EditError::Invalid("sample origin"))?;
    let size = u64::try_from(window.samples.len()).map_err(|_| EditError::Limit)?;
    let end = start.checked_add(size).ok_or(EditError::Limit)?;
    Ok((start, end))
}
