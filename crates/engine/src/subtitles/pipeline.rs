//! Orchestrates subtitle workflow stages.
use super::persistence::{
    burn_subtitles, caption_track_id, ensure_distinct, export_subtitles, resolve_path,
};
use super::{SubtitleOutcome, SubtitleRequest};
use crate::EditError;
use crate::cache::ArtifactCache;
use crate::jobs::JobRunner;
use crate::providers::{ConsentPrompt, Cost, Providers, normalize_target};
use crate::transcript::{
    Cancellation, Progress, Stage, build_session, caption_item_id, hex, media_duration, transcribe,
    translate_cues,
};
use captions::NfcText;
use media::SourceAsset;
use std::time::Instant;
use time::Tick;
/// Run the complete subtitle workflow against one input media file. The caller owns
/// the runner, cache and consent prompt; no project file is written.
pub fn run(
    request: &SubtitleRequest,
    providers: &Providers,
    runner: &mut JobRunner,
    cache: &mut ArtifactCache,
    consent: &dyn ConsentPrompt,
    progress: &dyn Progress,
    cancel: &Cancellation,
) -> Result<SubtitleOutcome, EditError> {
    let started = Instant::now();
    cancel.checkpoint()?;
    if request.srt.is_none() && request.burn.is_none() {
        return Err(EditError::Invalid("no subtitle output requested"));
    }
    let asset = SourceAsset::import(media::AssetId(1), request.input.clone())?;
    if asset
        .probe()
        .duration
        .is_none_or(|value| value <= Tick::ZERO)
    {
        return Err(EditError::Unsupported("media duration is unknown"));
    }
    if request.burn.is_some()
        && !asset
            .probe()
            .streams
            .iter()
            .any(|stream| stream.kind == media::StreamKind::Video)
    {
        return Err(EditError::Unsupported("burn-in requires a video stream"));
    }
    let duration = media_duration(&asset)?;
    // This retained blocking path owns admission; registry jobs declare the same
    // memory in their outer JobSpec and never enter this composition.
    let work = runner
        .memory_budget()
        .reserve(crate::ByteOwner::Compositor, crate::transcript::WORK_BYTES)?;
    let _result = runner.memory_budget().reserve(
        crate::ByteOwner::JobResults,
        crate::transcript::RESULT_BYTES,
    )?;
    let source_language = match &request.source_language {
        Some(language) => language.clone(),
        None => providers.detect_language(&asset, duration, cancel)?,
    };
    let target_language = normalize_target(&request.target_language)?;
    if request.bilingual && source_language == target_language {
        return Err(EditError::Unsupported(
            "bilingual output requires two distinct languages",
        ));
    }
    let need_translation = source_language != target_language;
    if need_translation && providers.translator().is_none() {
        return Err(EditError::Unsupported("translation provider is required"));
    }
    if !need_translation && request.bilingual {
        return Err(EditError::Unsupported(
            "bilingual output requires a translation target",
        ));
    }
    let transcription = transcribe(
        &asset,
        duration,
        &source_language,
        providers,
        None,
        &std::sync::Mutex::new(cache),
        consent,
        progress,
        cancel,
        cancel,
    )?;
    drop(work);
    let cues = transcription.cues;
    let transcription_cost = transcription.cost;
    let checkpoints = transcription.checkpoints;
    let segments = transcription.transcript.segments.len();
    let fingerprint = NfcText::from(hex(&asset.content_hash().0));
    cancel.checkpoint()?;
    let mut session = build_session(
        &asset,
        &cues,
        duration,
        request.burn.is_some(),
        request.portrait,
    )?;
    let caption_item = caption_item_id(&session)?;
    let caption_track = caption_track_id(&session)?;
    let mut translation_cost = Cost::Free;
    if need_translation {
        progress.stage(Stage::Translate);
        translate_cues(
            &mut session,
            providers,
            runner,
            consent,
            progress,
            cancel,
            &source_language,
            &target_language,
            &fingerprint,
            caption_item,
            &mut translation_cost,
        )?;
    }
    if let (Some(srt), Some(burn)) = (&request.srt, &request.burn)
        && resolve_path(srt)? == resolve_path(burn)?
    {
        return Err(EditError::Invalid("subtitle outputs must differ"));
    }
    let output_languages = if need_translation && request.bilingual {
        vec![source_language.clone(), target_language.clone()]
    } else {
        vec![target_language.clone()]
    };
    let srt = match &request.srt {
        Some(path) => {
            ensure_distinct(&request.input, path)?;
            cancel.checkpoint()?;
            progress.stage(Stage::ExportSrt);
            export_subtitles(
                &session,
                runner,
                progress,
                &asset,
                caption_track,
                &output_languages,
                path,
                request.overwrite,
                cancel,
            )?;
            Some(path.clone())
        }
        None => None,
    };
    let burn = match &request.burn {
        Some(path) => {
            ensure_distinct(&request.input, path)?;
            cancel.checkpoint()?;
            progress.stage(Stage::ExportBurn);
            burn_subtitles(
                &session,
                runner,
                progress,
                &asset,
                duration,
                &output_languages,
                path,
                request.portrait,
                request.overwrite,
                cancel,
            )?;
            Some(path.clone())
        }
        None => None,
    };
    Ok(SubtitleOutcome {
        source_language,
        target_language,
        transcription: providers.transcription().clone(),
        translation: providers.translation().cloned(),
        segments,
        cues: cues.len(),
        checkpoints,
        srt,
        burn,
        transcription_cost,
        translation_cost,
        elapsed: started.elapsed(),
    })
}
