//! Decodes the data a completed job returns through `job_status`.
use crate::jobs::JobKind;
use crate::session::wire::JobOutput;
use crate::{EditError, Result};

/// The output of a completed `kind` from the runner's retained bytes. The match names
/// every [`JobKind`], so a new kind does not compile until it states what it returns.
/// A kind that returns data and has lost its bytes refuses `JobNotRetained`.
pub(super) fn decode(kind: JobKind, bytes: Option<&[u8]>) -> Result<Option<JobOutput>> {
    let retained = || bytes.ok_or(EditError::JobNotRetained);
    Ok(Some(match kind {
        JobKind::Preview => JobOutput::Preview(
            serde_json::from_slice(retained()?).map_err(|_| EditError::InvalidParams)?,
        ),
        JobKind::AnalyzePauses => JobOutput::Pauses(crate::pause_candidates::decode(retained()?)?),
        JobKind::GenerateChapters => {
            JobOutput::Chapters(Box::new(crate::chapters::decode(retained()?)?))
        }
        JobKind::ClassifyFillers => JobOutput::Fillers {
            candidates: crate::fillers::decode(retained()?)?,
        },
        JobKind::MeasureLoudness => JobOutput::Loudness(crate::loudness::decode(retained()?)?),
        JobKind::DetectScenes => JobOutput::Scenes(crate::scene_detection::decode(retained()?)?),
        JobKind::Transcribe
        | JobKind::Translate
        | JobKind::SuggestClips
        | JobKind::AnalyzeShots
        | JobKind::AnalyzeSpeechActivity
        | JobKind::GenerateWaveform
        | JobKind::GenerateClipThumbnails
        | JobKind::EffectThumbnail
        | JobKind::Export
        | JobKind::Import
        | JobKind::Probe
        | JobKind::Thumbnail
        | JobKind::Persist
        | JobKind::DownloadModel
        | JobKind::SoundDownload
        | JobKind::DownloadFont => return Ok(None),
    }))
}
