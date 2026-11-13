//! Decodes the data a completed job returns through `job_status`.
use crate::jobs::JobKind;
use crate::session::wire::JobOutput;
use crate::{control::Error as EngineError, control::Result};

/// The output of a completed `kind` from the runner's retained bytes. The match names
/// every [`JobKind`], so a new kind does not compile until it states what it returns.
/// A kind that returns data and has lost its bytes refuses `JobNotRetained`.
pub(super) fn decode(kind: JobKind, bytes: Option<&[u8]>) -> Result<Option<JobOutput>> {
    let retained = || bytes.ok_or(EngineError::Runtime(crate::RuntimeError::JobNotRetained));
    Ok(Some(match kind {
        JobKind::ImportLut => {
            JobOutput::Lut(serde_json::from_slice(retained()?).map_err(|_| {
                EngineError::Control(crate::session::wire::ControlError::InvalidParams)
            })?)
        }
        JobKind::Transcribe => {
            JobOutput::Captions(Box::new(crate::subtitles::decode_captions(retained()?)?))
        }
        JobKind::Translate => JobOutput::Translations(Box::new(
            crate::subtitles::decode_translations(retained()?)?,
        )),
        JobKind::Preview => {
            JobOutput::Preview(serde_json::from_slice(retained()?).map_err(|_| {
                EngineError::Control(crate::session::wire::ControlError::InvalidParams)
            })?)
        }
        JobKind::AnalyzePauses => JobOutput::Pauses(crate::pause_candidates::decode(retained()?)?),
        JobKind::GenerateChapters => {
            JobOutput::Chapters(Box::new(crate::chapters::decode(retained()?)?))
        }
        JobKind::ClassifyFillers => JobOutput::Fillers {
            candidates: crate::fillers::decode(retained()?)?,
        },
        JobKind::SelectEmphasis => JobOutput::Emphasis {
            suggestions: crate::emphasis::decode(retained()?)?,
        },
        JobKind::MeasureLoudness => JobOutput::Loudness(crate::loudness::decode(retained()?)?),
        JobKind::DetectScenes => JobOutput::Scenes(crate::scene_detection::decode(retained()?)?),
        JobKind::AnalyzeReframe => JobOutput::Reframe(reframe_report(retained()?)?),
        JobKind::SuggestClips => {
            JobOutput::Clips(Box::new(super::clips::decode_clips(retained()?)?))
        }
        JobKind::AnalyzeShots
        | JobKind::AnalyzeFootage
        | JobKind::AnalyzeSpeechActivity
        | JobKind::GenerateWaveform
        | JobKind::GenerateClipThumbnails
        | JobKind::GenerateProxy
        | JobKind::EffectThumbnail
        | JobKind::AnimationPreview
        | JobKind::Export
        | JobKind::Import
        | JobKind::Probe
        | JobKind::Thumbnail
        | JobKind::Persist
        | JobKind::DownloadModel
        | JobKind::SoundDownload
        | JobKind::DownloadFont
        | JobKind::DownloadSticker => return Ok(None),
    }))
}

/// The small per-shot report and the retained analysis are one job output:
/// `reframe.apply` later parses the same bytes for the exact key set.
fn reframe_report(bytes: &[u8]) -> Result<crate::session::wire::ReframeReport> {
    let analysis: crate::ReframeAnalysis = serde_json::from_slice(bytes)
        .map_err(|_| EngineError::Control(crate::session::wire::ControlError::InvalidParams))?;
    Ok(crate::session::wire::ReframeReport {
        target: analysis.target.into(),
        size: analysis.size,
        shots: analysis
            .shots
            .iter()
            .map(|shot| crate::session::wire::ReframeShotReport {
                start: shot.range.start(),
                keys: shot.samples.len(),
                reasons: shot.reasons.iter().copied().map(Into::into).collect(),
            })
            .collect(),
    })
}

impl From<crate::ReframeTarget> for crate::session::wire::ReframeTarget {
    fn from(target: crate::ReframeTarget) -> Self {
        match target {
            crate::ReframeTarget::Vertical => Self::Vertical,
            crate::ReframeTarget::Square => Self::Square,
            crate::ReframeTarget::Landscape => Self::Landscape,
        }
    }
}

impl From<crate::session::wire::ReframeTarget> for crate::ReframeTarget {
    fn from(target: crate::session::wire::ReframeTarget) -> Self {
        match target {
            crate::session::wire::ReframeTarget::Vertical => Self::Vertical,
            crate::session::wire::ReframeTarget::Square => Self::Square,
            crate::session::wire::ReframeTarget::Landscape => Self::Landscape,
        }
    }
}

impl From<crate::ReframeReason> for crate::session::wire::ReframeReason {
    fn from(reason: crate::ReframeReason) -> Self {
        match reason {
            crate::ReframeReason::Ambiguous => Self::Ambiguous,
            crate::ReframeReason::NoSubject => Self::NoSubject,
            crate::ReframeReason::Lost => Self::Lost,
        }
    }
}
