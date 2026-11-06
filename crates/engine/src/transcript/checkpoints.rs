//! Reuses independently committed ASR windows.
use super::operation::{Cancellation, Progress, Stage};
use super::speech::{WORK_BYTES, sample_range};
use crate::EditError;
use crate::cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs};
use crate::jobs::{
    CancelToken, JobBytes, JobId, JobKind, JobOutput, JobRunner, JobSpec, JobState, ResourceClass,
};
use crate::providers::{ConsentPrompt, Cost, Providers, estimate_money};
use ai::{Request, SampleRange};
use captions::LanguageTag;
use captions::NfcText;
use media::SpeechWindow;
use std::{sync::Arc, time::Duration};
#[allow(clippy::too_many_arguments)]
pub(crate) fn transcribe_window(
    providers: &Providers,
    runner: &mut JobRunner,
    cache: &mut ArtifactCache,
    consent: &dyn ConsentPrompt,
    progress: &dyn Progress,
    cancel: &Cancellation,
    language: &LanguageTag,
    fingerprint: &NfcText,
    window: SpeechWindow,
    cost: &mut Cost,
    checkpoints: &mut usize,
) -> Result<ai::Recognition, EditError> {
    cancel.checkpoint()?;
    let (start, end) = sample_range(&window)?;
    let samples = SampleRange::new(start, end)?;
    let sample_count = u64::try_from(window.samples.len()).map_err(|_| EditError::Limit)?;
    let samples_pcm = Arc::new(window.samples);
    let scope = providers.transcription_scope(language, samples, sample_count, fingerprint, 0)?;
    let config = providers.transcriber().config();
    let key = CacheKey::new(KeyInputs {
        kind: ArtifactKind::AsrChunk,
        source: *blake3::hash(
            format!("{}:{}:{}", hex(fingerprint.as_str().as_bytes()), start, end).as_bytes(),
        )
        .as_bytes(),
        provider: config.id().as_str(),
        model: config.model().as_str(),
        version: config.settings_fingerprint().as_str(),
        settings: format!("language={};samples={start}..{end}", language.as_str()).as_bytes(),
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })?;
    let cached = match cache.get(key) {
        Ok(value) => value,
        Err(EditError::CacheCorrupt) => None,
        Err(error) => return Err(error),
    };
    let raw = match cached {
        Some(lease) => {
            *checkpoints = checkpoints.saturating_add(1);
            serde_json::from_slice(lease.bytes())
                .map_err(|_| EditError::Invalid("cached recognition is malformed"))?
        }
        None => {
            let estimate = providers.transcriber().estimate(&scope);
            cost.add(&estimate);
            let consent_token = if providers.transcription().is_remote() {
                let request = providers.upload_request(scope.clone())?;
                consent.approve(&request)?;
                Some(ai::Consent::grant(
                    scope.clone(),
                    estimate_money(&estimate)?,
                )?)
            } else {
                None
            };
            let provider = providers.transcriber_arc();
            let job_scope = scope.clone();
            let job_samples = samples_pcm.clone();
            let (id, token) = runner.submit(
                JobSpec {
                    kind: JobKind::Transcribe,
                    class: ResourceClass::Network,
                    input: key,
                    bytes: JobBytes {
                        compositor: WORK_BYTES,
                        result: 16 * 1024 * 1024,
                        ..Default::default()
                    },
                },
                move |hooks| {
                    let audio = ai::AudioWindow {
                        samples: job_samples.as_slice(),
                        range: job_scope.source_samples,
                        sample_rate: job_scope.sample_rate,
                    };
                    let request = Request {
                        scope: &job_scope,
                        consent: consent_token.as_ref(),
                    };
                    let recognition =
                        ai::transcribe(provider.as_ref(), audio, &request, None, hooks)?;
                    serde_json::to_vec(&recognition)
                        .map_err(|_| EditError::Invalid("recognition serialization"))
                },
            )?;
            let output = wait_for_job(runner, id, &token, progress, cancel)?;
            let bytes = output.bytes().to_vec();
            let recognition: ai::Recognition = serde_json::from_slice(&bytes)
                .map_err(|_| EditError::Invalid("recognition response is malformed"))?;
            match cache.put(key, &bytes) {
                Ok(()) => {}
                // A checkpoint that cannot be stored must not discard a valid recognition.
                Err(EditError::BudgetFull | EditError::ArtifactTooLarge) => {}
                Err(error) => return Err(error),
            }
            if cancel.cancelled() {
                return Err(EditError::Cancelled);
            }
            return align_recognition(
                providers,
                runner,
                progress,
                cancel,
                language,
                fingerprint,
                &samples_pcm,
                samples,
                recognition,
            );
        }
    };
    align_recognition(
        providers,
        runner,
        progress,
        cancel,
        language,
        fingerprint,
        &samples_pcm,
        samples,
        raw,
    )
}

#[allow(clippy::too_many_arguments)]
fn align_recognition(
    providers: &Providers,
    runner: &mut JobRunner,
    progress: &dyn Progress,
    cancel: &Cancellation,
    language: &LanguageTag,
    fingerprint: &NfcText,
    samples_pcm: &Arc<Vec<f32>>,
    samples: SampleRange,
    recognition: ai::Recognition,
) -> Result<ai::Recognition, EditError> {
    let Some(aligner) = providers.aligner_arc() else {
        return Ok(recognition);
    };
    let unit = providers.language_profile(language)?.timing_unit;
    if !recognition.needs_timing(unit) {
        return Ok(recognition);
    }
    cancel.checkpoint()?;
    progress.stage(Stage::Align);
    let scope = providers.alignment_scope(language, samples, fingerprint)?;
    let samples_pcm = samples_pcm.clone();
    let aligner_job = aligner.clone();
    let scope_job = scope.clone();
    let (id, token) = runner.submit(
        JobSpec {
            kind: JobKind::Transcribe,
            class: ResourceClass::Cpu,
            input: CacheKey::new(KeyInputs {
                kind: ArtifactKind::AsrChunk,
                source: *blake3::hash(
                    format!(
                        "align:{}:{}:{}",
                        fingerprint.as_str(),
                        samples.start(),
                        samples.end()
                    )
                    .as_bytes(),
                )
                .as_bytes(),
                provider: aligner.config().id().as_str(),
                model: aligner.config().model().as_str(),
                version: aligner.config().settings_fingerprint().as_str(),
                settings: format!("language={}", language.as_str()).as_bytes(),
                transcript: [0; 32],
                cues: [0; 32],
                clip_range: [0; 32],
                translation: [0; 32],
            })?,
            bytes: JobBytes {
                compositor: WORK_BYTES,
                result: 16 * 1024 * 1024,
                ..Default::default()
            },
        },
        move |hooks| {
            let audio = ai::AudioWindow {
                samples: samples_pcm.as_slice(),
                range: scope_job.source_samples,
                sample_rate: scope_job.sample_rate,
            };
            let aligned = ai::align(
                recognition.clone(),
                aligner_job.as_ref(),
                audio,
                &scope_job,
                None,
                unit,
                hooks,
            )?;
            serde_json::to_vec(&aligned)
                .map_err(|_| EditError::Invalid("aligned recognition serialization"))
        },
    )?;
    let output = wait_for_job(runner, id, &token, progress, cancel)?;
    if cancel.cancelled() {
        return Err(EditError::Cancelled);
    }
    serde_json::from_slice(output.bytes())
        .map_err(|_| EditError::Invalid("aligned recognition is malformed"))
}

/// Pump one submitted job to its terminal state, forwarding progress and cancellation.
/// Completed publication wins a late cancellation; failed jobs retain their typed source.
pub fn wait_for_job(
    runner: &mut JobRunner,
    id: JobId,
    token: &CancelToken,
    progress: &dyn Progress,
    cancel: &Cancellation,
) -> Result<JobOutput, EditError> {
    loop {
        if cancel.cancelled() {
            token.cancel();
        }
        runner.tick()?;
        for event in runner.drain_events() {
            if event.id == id {
                progress.job(&event);
            }
        }
        let snapshot = runner.snapshot(id).ok_or(EditError::StaleResult)?;
        match snapshot.state {
            // A completed job is returned even if cancellation arrived during its
            // publication fence; callers decide whether later stages may proceed.
            JobState::Completed => return runner.take_output(id).ok_or(EditError::StaleResult),
            JobState::Failed => {
                return Err(match snapshot.error {
                    Some(error) => EditError::JobFailure(error),
                    None => EditError::Invalid("job failed without diagnostics"),
                });
            }
            JobState::Cancelled => return Err(EditError::Cancelled),
            _ => {
                if cancel.cancelled() {
                    token.cancel();
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(
            DIGITS.get(usize::from(byte >> 4)).copied().unwrap_or(b'0'),
        ));
        output.push(char::from(
            DIGITS
                .get(usize::from(byte & 0x0f))
                .copied()
                .unwrap_or(b'0'),
        ));
    }
    output
}
