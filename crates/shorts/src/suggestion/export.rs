//! Exports accepted clip candidates.
use super::{
    Cancellation, ClipBatch, ClipBatchRequest, ClipExportOutcome, ClipExportPaths, ClipExportPrep,
    ClipExportRequest, ClipShort, ClipSuggestionOutcome, ConsentPrompt, Cost, CueTrack, Duration,
    EditError, Error, JobRunner, JobState, MAX_BATCH_CLIPS, NfcText, Progress, Providers,
    ResourceClass, fingerprint, translate_track,
};
/// Translate (when needed) and prepare every selected short for export.
#[allow(clippy::too_many_arguments)]
pub fn prepare_export(
    outcome: &ClipSuggestionOutcome,
    request: ClipExportRequest,
    providers: &Providers,
    runner: &mut JobRunner,
    consent: &dyn ConsentPrompt,
    progress: &dyn Progress,
    cancel: &Cancellation,
) -> Result<ClipExportPrep, Error> {
    if request.select.is_empty() || request.select.len() > MAX_BATCH_CLIPS {
        return Err(EditError::Invalid("clip selection size").into());
    }
    let mut seen = std::collections::BTreeSet::new();
    for index in &request.select {
        if *index >= outcome.clips.len() || !seen.insert(*index) {
            return Err(EditError::Invalid("clip selection index").into());
        }
    }
    let mut track = CueTrack {
        styles: Vec::new(),
        cues: outcome.source_cues.clone(),
    };
    let mut translation_cost = Cost::Free;
    for language in &request.languages {
        if language == &outcome.source_language {
            continue;
        }
        let translated = translate_track(
            &outcome.asset,
            outcome.duration,
            &outcome.source_cues,
            providers,
            runner,
            consent,
            progress,
            cancel,
            &outcome.source_language,
            language,
            &NfcText::from(fingerprint(&outcome.asset)),
            &mut translation_cost,
        )?;
        for (target, source) in track.cues.iter_mut().zip(translated.cues) {
            if target.id != source.id {
                return Err(EditError::Invalid("translated clip cues").into());
            }
            for (tag, text) in source.texts {
                if tag != outcome.source_language {
                    target.texts.insert(tag, text);
                }
            }
        }
    }
    let mut shorts = Vec::with_capacity(request.select.len());
    for index in &request.select {
        let candidate = outcome
            .clips
            .get(*index)
            .ok_or(EditError::Invalid("clip selection index"))?;
        shorts.push(ClipShort::new(
            candidate.proposal.title.clone(),
            candidate.segments.clone(),
        ));
    }
    let mut batch = ClipBatchRequest::new(outcome.asset.id(), shorts, request.directory);
    batch.cues = track;
    batch.languages = request.languages;
    batch.subtitle_formats = request.formats;
    batch.overwrite = request.overwrite;
    batch.allow_degraded = request.allow_degraded;
    batch.max_output_bytes = request.max_output_bytes;
    let batch = ClipBatch::prepare(&outcome.source_project, batch)?;
    Ok(ClipExportPrep {
        batch,
        selected: request.select,
        translation_cost,
    })
}

/// Export with bounded parallelism, refilling slots as shorts complete. A failed
/// admission or job cancels and drains admitted siblings before returning.
/// Already published complete files remain.
#[allow(clippy::too_many_arguments)]
pub fn export(
    outcome: &ClipSuggestionOutcome,
    request: ClipExportRequest,
    providers: &Providers,
    runner: &mut JobRunner,
    consent: &dyn ConsentPrompt,
    progress: &dyn Progress,
    cancel: &Cancellation,
) -> Result<ClipExportOutcome, Error> {
    let prep = prepare_export(
        outcome, request, providers, runner, consent, progress, cancel,
    )?;
    let mut batch = prep.batch;
    let capacity = runner.batch_capacity(ResourceClass::GpuEncoder);
    let mut active = Vec::with_capacity(capacity);
    let mut next = 0;
    let result = (|| {
        while next < prep.selected.len() || !active.is_empty() {
            cancel.checkpoint()?;
            while active.len() < capacity && next < prep.selected.len() {
                cancel.checkpoint()?;
                match batch.submit_one(runner, next) {
                    Ok(_) => {
                        active.push(next);
                        next += 1;
                    }
                    // Pump the caller's other work before retrying admission.
                    Err(Error::Engine(EditError::Runtime(engine::RuntimeError::QueueFull))) => {
                        break;
                    }
                    Err(error) => return Err(error),
                }
            }
            cancel.checkpoint()?;
            runner.tick()?;
            for event in runner.drain_events() {
                if active
                    .iter()
                    .any(|slot| batch.job_id(*slot) == Some(event.id))
                {
                    progress.job(&event);
                }
            }
            let mut index = 0;
            while index < active.len() {
                let slot = *active.get(index).ok_or(EditError::StaleResult)?;
                let snapshot = batch.snapshot(runner, slot).ok_or(EditError::StaleResult)?;
                match snapshot.state {
                    JobState::Completed => {
                        let _ = runner
                            .take_output(snapshot.id)
                            .ok_or(EditError::StaleResult)?;
                        active.swap_remove(index);
                    }
                    JobState::Failed => {
                        return Err(match snapshot.error {
                            Some(error) => {
                                EditError::Runtime(engine::RuntimeError::Failed(error)).into()
                            }
                            None => {
                                EditError::Invalid("clip job failed without diagnostics").into()
                            }
                        });
                    }
                    JobState::Cancelled => {
                        return Err(EditError::Runtime(engine::RuntimeError::Cancelled).into());
                    }
                    _ => index += 1,
                }
            }
            if !active.is_empty() || next < prep.selected.len() {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        batch.cancel_all();
        // Keep the batch alive until every admitted worker has reached a terminal
        // state. Never shut down unrelated work in the caller's runner.
        loop {
            let running = (0..next).any(|slot| {
                batch.snapshot(runner, slot).is_some_and(|snapshot| {
                    !matches!(
                        snapshot.state,
                        JobState::Completed | JobState::Failed | JobState::Cancelled
                    )
                })
            });
            if !running {
                break;
            }
            runner.tick()?;
            std::thread::sleep(Duration::from_millis(5));
        }
        for slot in 0..next {
            if let Some(id) = batch.job_id(slot) {
                let _ = runner.take_output(id);
            }
        }
        return Err(error);
    }
    let mut exports = Vec::with_capacity(prep.selected.len());
    for (slot, candidate) in prep.selected.iter().enumerate() {
        exports.push(ClipExportPaths {
            candidate: *candidate,
            paths: batch
                .paths(slot)
                .ok_or(EditError::Invalid("clip paths"))?
                .clone(),
        });
    }
    Ok(ClipExportOutcome {
        exports,
        translation_cost: prep.translation_cost,
    })
}
