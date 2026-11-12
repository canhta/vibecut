//! Produces source-snapped clip candidates.
use super::{
    BoundaryFailure, ClipBoundarySettings, ClipCandidate, ClipInput, ClipSegment, ClipSettings,
    ClipSuggestion, Cue, EngineError, Error, FrameRate, LanguageTag, SnapSettings, SourceAsset,
    SourceRange, Tick, snap_clip_boundaries,
};
pub(super) fn clip_input(
    cues: &[Cue],
    language: &LanguageTag,
    settings: &ClipSettings,
) -> Result<ClipInput, EngineError> {
    let mut segments = Vec::new();
    segments
        .try_reserve_exact(cues.len())
        .map_err(|_| EngineError::Control(engine::control::ControlError::Limit))?;
    for cue in cues {
        let text = cue
            .texts
            .get(language)
            .ok_or(EngineError::Edit(engine::EditError::Invalid(
                "clip transcript language",
            )))?;
        segments.push(ClipSegment {
            id: cue.id.clone(),
            text: text.text.clone(),
            range: cue.range,
        });
    }
    let input = ClipInput {
        segments,
        settings: settings.clone(),
    };
    input.validate()?;
    Ok(input)
}

pub(super) fn snap_settings(
    asset: &SourceAsset,
    duration: Tick,
    settings: &ClipSettings,
    snap: &SnapSettings,
) -> Result<ClipBoundarySettings, EngineError> {
    let frame_rate = snap.frame_rate.or_else(|| {
        asset
            .probe()
            .streams
            .iter()
            .find(|stream| stream.kind == media::StreamKind::Video)
            .and_then(|stream| stream.frame_rate)
    });
    Ok(ClipBoundarySettings {
        source_range: SourceRange::new(Tick::ZERO, duration)?,
        frame_rate: frame_rate.unwrap_or(FrameRate::FPS_30),
        tolerance: snap.tolerance,
        silence_rms: snap.silence_rms,
        min_duration: settings.min_duration,
        max_duration: settings.max_duration,
        min_segment_duration: settings.min_segment_duration,
        max_segments: settings.max_segments,
    })
}

pub(super) fn snap_candidate(
    suggestion: &ClipSuggestion,
    transcript: &captions::Transcript,
    profile: &captions::LanguageProfile,
    energy: &[media::SpeechEnergyWindow],
    settings: &ClipBoundarySettings,
) -> Result<ClipCandidate, BoundaryFailure> {
    let snapped = snap_clip_boundaries(
        std::slice::from_ref(&suggestion.segments),
        transcript,
        profile,
        energy,
        settings,
    )
    .map_err(|error| match error {
        Error::ClipBoundary { reason, .. } => reason,
        _ => BoundaryFailure::NoSafeFrame,
    })?;
    let Some(short) = snapped.into_iter().next() else {
        return Err(BoundaryFailure::Candidate);
    };
    Ok(ClipCandidate {
        proposal: suggestion.proposal.clone(),
        segments: short.segments,
        adjustments: short.adjustments,
        duration: short.duration,
        source: suggestion.source,
    })
}

pub(super) fn fingerprint(asset: &SourceAsset) -> String {
    format!(
        "clips:{}",
        blake3::Hash::from(asset.content_hash().0).to_hex()
    )
}
