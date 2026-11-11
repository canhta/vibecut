//! Submits source-asset transcription as one bounded workflow job.
use super::{CaptionAccess, outputs::MAX_RESULT_BYTES};
use crate::jobs::{JobBytes, JobId, JobKind, JobRunner, JobSpec, ResourceClass};
use crate::transcript::{Cancellation, Progress, Stage};
use crate::{EditError, Result, Session, session::wire::CaptionsResult};
use captions::LanguageTag;
use time::{SourceRange, Tick};

pub(crate) fn generate(
    session: &Session,
    runner: &mut JobRunner,
    access: &CaptionAccess,
    asset: media::AssetId,
    range: Option<SourceRange>,
    language: Option<LanguageTag>,
    input: crate::cache::CacheKey,
) -> Result<JobId> {
    let asset = session
        .project()
        .data()
        .assets
        .iter()
        .find(|a| a.id() == asset)
        .cloned()
        .ok_or(EditError::NoAsset(asset))?;
    let duration = crate::transcript::media_duration(&asset)?;
    let range = range.unwrap_or(SourceRange::new(Tick::ZERO, duration)?);
    if range.start() < Tick::ZERO || range.end() > duration {
        return Err(EditError::Invalid("transcription range outside asset"));
    }
    let (providers, consent) = access.share()?;
    let cache = access.cache()?.clone();
    let class = if providers.transcriber().config().is_remote() && providers.aligner_arc().is_none()
    {
        ResourceClass::Network
    } else {
        ResourceClass::Cpu
    };
    let (id, _) = runner.submit_for_session(
        session,
        JobSpec {
            kind: JobKind::Transcribe,
            class,
            input,
            bytes: JobBytes {
                compositor: crate::transcript::WORK_BYTES,
                result: MAX_RESULT_BYTES as u64,
                ..Default::default()
            },
        },
        move |hooks| {
            let cancel = Cancellation::from_flag(hooks.media_cancel_flag());
            hooks.check_cancelled()?;
            validate_source(&asset)?;
            hooks.stage(Stage::Extract);
            let language = match &language {
                Some(language) => language.clone(),
                None => providers.detect_language_with_hooks(&asset, duration, &cancel, hooks)?,
            };
            let output = crate::transcript::transcribe(
                &asset,
                duration,
                &language,
                &providers,
                Some(range),
                &cache,
                consent.as_ref(),
                hooks,
                &cancel,
                hooks,
            )?;
            hooks.check_cancelled()?;
            validate_source(&asset)?;
            let config = providers.transcriber().config();
            let result = CaptionsResult {
                asset: asset.id(),
                range,
                source_language: language,
                segments: output.transcript.segments.len(),
                checkpoints: output.checkpoints,
                cost: (&output.cost).into(),
                cues: output.cues,
                provider: config.id().clone(),
                model: config.model().clone(),
            };
            if result.cues.len() > super::outputs::MAX_CUES {
                return Err(EditError::Limit);
            }
            crate::session::wire::bounded_json(&result, MAX_RESULT_BYTES)
        },
    )?;
    Ok(id)
}

pub(crate) fn validate_source(asset: &media::SourceAsset) -> Result<()> {
    let current = media::SourceAsset::import(asset.id(), asset.path().to_owned())?;
    if current.content_hash() != asset.content_hash() || current.probe() != asset.probe() {
        return Err(EditError::Session(crate::SessionError::StaleResult));
    }
    Ok(())
}
