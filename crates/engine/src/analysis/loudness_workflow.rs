//! Two-pass EBU R128 normalization: content-bound gains, a measurement job over
//! the production mix graph, and export admission. The job never edits the
//! project; the caller applies its gains through `audio.apply_loudness`.
use crate::runtime::{LOUDNESS_VERSION as VERSION, loudness_content_key as content_key};
use crate::session::wire::LoudnessResult;
use crate::{
    EditError,
    cache::{ArtifactKind, CacheKey, KeyInputs},
    jobs::{JobBytes, JobId, JobKind, JobRunner, JobSpec, ResourceClass},
};
use project::Project;
use std::sync::Arc;

const MAX_BYTES: usize = 1 << 20;

impl From<render::Measurement> for crate::session::wire::Measured {
    fn from(m: render::Measurement) -> Self {
        Self {
            integrated_lufs: m.integrated_lufs,
            true_peak_db: m.true_peak_db,
        }
    }
}

fn job_key(content: [u8; 32]) -> Result<CacheKey, EditError> {
    CacheKey::new(KeyInputs {
        kind: ArtifactKind::Loudness,
        source: content,
        provider: "local",
        model: "ebur128",
        version: VERSION,
        settings: &[],
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
}

/// Queue one CPU measurement of the project's requested targets. The result is
/// retained by the runner; `job_status` returns it and `audio.apply_loudness`
/// applies its gains while the content is unchanged.
pub fn submit_loudness(
    runner: &mut JobRunner,
    project: Arc<Project>,
    sources: impl Fn() -> Box<dyn render::SourceProvider> + Send + Sync + 'static,
) -> Result<JobId, EditError> {
    let l = &project.data().settings.loudness;
    if l.clip.is_none() && l.master.is_none() {
        return Err(EditError::Invalid("no loudness target set"));
    }
    let content = content_key(project.data());
    let (id, _) = runner.submit(
        JobSpec {
            kind: JobKind::MeasureLoudness,
            class: ResourceClass::Cpu,
            input: job_key(content)?,
            bytes: JobBytes {
                result: MAX_BYTES as u64,
                ..JobBytes::default()
            },
        },
        move |context| {
            context.check_cancelled()?;
            let report = render::measure_loudness(&project, sources().as_mut(), context)?;
            let mut gains = report.gains;
            gains.key = content;
            let result = LoudnessResult {
                gains,
                clips: report
                    .clips
                    .into_iter()
                    .map(|(i, m)| (i, m.into()))
                    .collect(),
                master: report.master.map(Into::into),
            };
            context.check_cancelled()?;
            let bytes = serde_json::to_vec(&result).map_err(|_| EditError::Limit)?;
            if bytes.len() > MAX_BYTES {
                return Err(EditError::Limit);
            }
            Ok(bytes)
        },
    )?;
    Ok(id)
}

/// Decode and validate retained job bytes.
pub fn decode(bytes: &[u8]) -> Result<LoudnessResult, EditError> {
    if bytes.len() > MAX_BYTES {
        return Err(EditError::Invalid("loudness result"));
    }
    let result: LoudnessResult =
        serde_json::from_slice(bytes).map_err(|_| EditError::Invalid("loudness result"))?;
    if !result.gains.is_valid() {
        return Err(EditError::Invalid("loudness result"));
    }
    Ok(result)
}
