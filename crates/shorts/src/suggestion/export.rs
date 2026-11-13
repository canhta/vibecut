//! Submits the accepted candidates of a suggestion as one export job per short.
use super::segments_from_view;
use crate::error::Error;
use crate::{ClipBatch, ClipBatchRequest, ClipShort, MAX_BATCH_CLIPS};
use engine::control::{ClipExportShort, ClipSubtitleFile, ClipsExportRequest};
use engine::jobs::JobRunner;
use project::Project;
use std::collections::BTreeSet;

/// Per-artifact output and staging ceiling of the registry export, as the CLI always used.
const MAX_OUTPUT_BYTES: u64 = 64 << 30;

fn invalid(reason: &'static str) -> Error {
    engine::EditError::Invalid(reason).into()
}

/// Prepare every selected short against `project` and submit one job per short onto `runner`.
///
/// `ClipBatch` is the one export path: it owns naming, caption mapping, portrait crops,
/// publication and per-short cancellation. A failed admission cancels the shorts already
/// admitted before returning its typed error; shorts that already published stay complete.
/// The caller sizes the runner's queue for the whole selection.
pub(crate) fn submit_export(
    runner: &mut JobRunner,
    project: &Project,
    request: ClipsExportRequest,
) -> Result<Vec<ClipExportShort>, Error> {
    let select = request.select;
    if select.is_empty() || select.len() > MAX_BATCH_CLIPS {
        return Err(invalid("clip selection size"));
    }
    let mut seen = BTreeSet::new();
    let mut shorts = Vec::with_capacity(select.len());
    for index in &select {
        let candidate = request
            .result
            .clips
            .get(*index)
            .filter(|_| seen.insert(*index))
            .ok_or(invalid("clip selection index"))?;
        shorts.push(ClipShort::new(
            candidate.title.clone(),
            segments_from_view(candidate)?,
        ));
    }
    let mut batch = ClipBatchRequest::new(request.result.asset, shorts, request.directory);
    batch.cues = request.cues;
    batch.shots = request.shots;
    batch.languages = request.languages;
    batch.subtitle_formats = request.formats;
    batch.overwrite = request.overwrite;
    batch.allow_degraded = request.allow_degraded;
    batch.max_output_bytes = MAX_OUTPUT_BYTES;
    let mut batch = ClipBatch::prepare(project, batch)?;
    for slot in 0..select.len() {
        if let Err(error) = batch.submit_one(runner, slot) {
            batch.cancel_all();
            return Err(error);
        }
    }
    let mut accepted = Vec::with_capacity(select.len());
    for (slot, candidate) in select.iter().enumerate() {
        let (Some(job), Some(paths)) = (batch.job_id(slot), batch.paths(slot)) else {
            batch.cancel_all();
            return Err(invalid("clip export job"));
        };
        accepted.push(ClipExportShort {
            candidate: *candidate,
            job,
            video: paths.video.clone(),
            subtitles: paths
                .subtitles
                .iter()
                .map(|subtitle| ClipSubtitleFile {
                    format: subtitle.format,
                    path: subtitle.path.clone(),
                })
                .collect(),
        });
    }
    Ok(accepted)
}
