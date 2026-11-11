//! Reuses independently committed ASR windows.
use super::operation::{Cancellation, Progress, Stage};
use super::speech::sample_range;
use crate::Error as EngineError;
use crate::cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs};
use crate::jobs::{CancelToken, JobId, JobOutput, JobRunner, JobState};
use crate::providers::{ConsentPrompt, Cost, Providers, approve_upload};
use ai::{ExecutionHooks, Request, SampleRange};
use captions::{LanguageTag, NfcText};
use media::SpeechWindow;
use std::{borrow::BorrowMut, sync::Mutex, time::Duration};

#[allow(clippy::too_many_arguments)]
pub(crate) fn transcribe_window<C: BorrowMut<ArtifactCache>>(
    providers: &Providers,
    cache: &Mutex<C>,
    consent: &dyn ConsentPrompt,
    progress: &dyn Progress,
    hooks: &dyn ExecutionHooks,
    language: &LanguageTag,
    fingerprint: &NfcText,
    window: SpeechWindow,
    cost: &mut Cost,
    checkpoints: &mut usize,
) -> Result<ai::Recognition, EngineError> {
    if hooks.cancelled() {
        return Err(EngineError::Runtime(crate::RuntimeError::Cancelled));
    }
    let (start, end) = sample_range(&window)?;
    let samples = SampleRange::new(start, end)?;
    let scope = providers.transcription_scope(language, samples, fingerprint)?;
    let config = providers.transcriber().config();
    // This is the original raw-window identity; job identity never replaces it.
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
    let cached = {
        let mut guard = cache.lock().map_err(|_| crate::RuntimeError::Poisoned)?;
        match (*guard).borrow_mut().get(key) {
            Ok(value) => value,
            Err(EngineError::Runtime(crate::RuntimeError::CacheCorrupt)) => None,
            Err(error) => return Err(error),
        }
    };
    let raw = match cached {
        Some(lease) => {
            *checkpoints = checkpoints.saturating_add(1);
            serde_json::from_slice(lease.bytes()).map_err(|_| {
                EngineError::Edit(crate::EditError::Invalid("cached recognition is malformed"))
            })?
        }
        None => {
            let estimate = providers.transcriber().estimate(&scope);
            cost.add(&estimate);
            let token = if config.is_remote() {
                let request = providers.upload_request(scope.clone())?;
                Some(approve_upload(consent, &request, &scope, &estimate)?)
            } else {
                None
            };
            let recognition = ai::transcribe(
                providers.transcriber(),
                ai::AudioWindow {
                    samples: &window.samples,
                    range: samples,
                    sample_rate: scope.sample_rate,
                },
                &Request {
                    scope: &scope,
                    consent: token.as_ref(),
                },
                None,
                hooks,
            )
            .map_err(provider_error)?;
            let bytes = crate::session::wire::bounded_json(&recognition, 16 * 1024 * 1024)?;
            // Completed recognition is durable before alignment or another cancellation check.
            {
                let mut guard = cache.lock().map_err(|_| crate::RuntimeError::Poisoned)?;
                match (*guard).borrow_mut().put(key, &bytes) {
                    Ok(()) => {}
                    Err(EngineError::Runtime(
                        crate::RuntimeError::BudgetFull | crate::RuntimeError::ArtifactTooLarge,
                    )) => {}
                    Err(error) => return Err(error),
                }
            }
            recognition
        }
    };
    if hooks.cancelled() {
        return Err(EngineError::Runtime(crate::RuntimeError::Cancelled));
    }
    let Some(aligner) = providers.aligner_arc() else {
        return Ok(raw);
    };
    let unit = providers.language_profile(language)?.timing_unit;
    if !raw.needs_timing(unit) {
        return Ok(raw);
    }
    progress.stage(Stage::Align);
    let scope = providers.alignment_scope(language, samples, fingerprint)?;
    Ok(ai::align(
        raw,
        aligner.as_ref(),
        ai::AudioWindow {
            samples: &window.samples,
            range: samples,
            sample_rate: scope.sample_rate,
        },
        &scope,
        None,
        unit,
        hooks,
    )
    .map_err(provider_error)?)
}

pub(super) fn provider_error(error: ai::Error) -> EngineError {
    match error {
        ai::Error::Cancelled => EngineError::Runtime(crate::RuntimeError::Cancelled),
        error => error.into(),
    }
}

/// Pump one submitted job to its terminal state, forwarding progress and cancellation.
/// Completed publication wins a late cancellation; failed jobs retain their typed source.
pub fn wait_for_job(
    runner: &mut JobRunner,
    id: JobId,
    token: &CancelToken,
    progress: &dyn Progress,
    cancel: &Cancellation,
) -> Result<JobOutput, EngineError> {
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
        let snapshot = runner
            .snapshot(id)
            .ok_or(EngineError::Session(crate::SessionError::StaleResult))?;
        match snapshot.state {
            // A completed job is returned even if cancellation arrived during its
            // publication fence; callers decide whether later stages may proceed.
            JobState::Completed => {
                return runner
                    .take_output(id)
                    .ok_or(EngineError::Session(crate::SessionError::StaleResult));
            }
            JobState::Failed => {
                return Err(match snapshot.error {
                    Some(error) => EngineError::Runtime(crate::RuntimeError::Failed(error)),
                    None => EngineError::Edit(crate::EditError::Invalid(
                        "job failed without diagnostics",
                    )),
                });
            }
            JobState::Cancelled => {
                return Err(EngineError::Runtime(crate::RuntimeError::Cancelled));
            }
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
