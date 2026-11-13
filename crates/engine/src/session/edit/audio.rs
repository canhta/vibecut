//! Applies audio commands to a candidate project.
use super::{EditError, ItemId, ItemParams, ProjectData, Result, Tick, item};

/// Cleanup chain of an audible item.
pub(super) fn cleanup_mut(content: &mut ItemParams) -> Option<&mut project::AudioCleanup> {
    match content {
        ItemParams::Audio(project::AudioParams { cleanup, .. })
        | ItemParams::Video(project::VideoParams { cleanup, .. }) => Some(cleanup),
        _ => None,
    }
}
/// Fades of an audible item.
pub(super) fn fades_mut(content: &mut ItemParams) -> Option<&mut project::Fades> {
    match content {
        ItemParams::Audio(project::AudioParams { fades, .. })
        | ItemParams::Video(project::VideoParams { fades, .. }) => Some(fades),
        _ => None,
    }
}
pub(super) fn execute_set_master(
    mut data: ProjectData,
    volume: f32,
    muted: bool,
) -> Result<ProjectData> {
    if !project::valid_volume(volume) {
        return Err(EditError::Invalid("master volume"));
    }
    data.settings.master_volume = volume;
    data.settings.master_muted = muted;

    Ok(data)
}
pub(super) fn execute_set_fade(
    mut data: ProjectData,
    id: ItemId,
    edge: project::FadeEdge,
    duration: Tick,
    curve: project::FadeCurve,
) -> Result<ProjectData> {
    let target = item(&mut data, id)?;
    let clip = target.duration()?;
    let Some(fades) = fades_mut(&mut target.content) else {
        return Err(EditError::Invalid("fade needs an audio or video item"));
    };
    fades.set(edge, duration, curve, clip)?;

    Ok(data)
}
pub(super) fn execute_set_loudness(
    mut data: ProjectData,
    scope: project::LoudnessScope,
    target_lufs: f32,
    true_peak_db: f32,
) -> Result<ProjectData> {
    let target = project::LoudnessTarget {
        target_lufs,
        true_peak_db,
    };
    if !target.is_valid() {
        return Err(EditError::Invalid("loudness target"));
    }
    let l = &mut data.settings.loudness;
    match scope {
        project::LoudnessScope::Clip => l.clip = Some(target),
        project::LoudnessScope::Master => l.master = Some(target),
    }
    // Gains were measured for the previous targets; remeasure before export.
    l.measured = None;

    Ok(data)
}
pub(super) fn execute_clear_loudness(
    mut data: ProjectData,
    scope: project::LoudnessScope,
) -> Result<ProjectData> {
    let l = &mut data.settings.loudness;
    let target = match scope {
        project::LoudnessScope::Clip => &mut l.clip,
        project::LoudnessScope::Master => &mut l.master,
    };
    *target = None;
    l.measured = None;

    Ok(data)
}
pub(super) fn execute_apply_loudness(
    mut data: ProjectData,
    gains: project::LoudnessGains,
) -> Result<ProjectData> {
    if !gains.is_valid() {
        return Err(EditError::Invalid("loudness gains"));
    }
    data.settings.loudness.measured = Some(gains);

    Ok(data)
}
pub(super) fn execute_add_cleanup_effect(
    mut data: ProjectData,
    id: ItemId,
    effect: project::CleanupEffect,
) -> Result<ProjectData> {
    let target = item(&mut data, id)?;
    let Some(cleanup) = cleanup_mut(&mut target.content) else {
        return Err(EditError::Invalid("cleanup needs an audio or video item"));
    };
    if cleanup.effects.iter().any(|other| other.id == effect.id) {
        return Err(EditError::Invalid("cleanup effect id"));
    }
    cleanup.effects.push(effect);
    cleanup.validate()?;

    Ok(data)
}
pub(super) fn execute_update_cleanup_effect(
    mut data: ProjectData,
    id: ItemId,
    effect: project::CleanupEffect,
) -> Result<ProjectData> {
    let target = item(&mut data, id)?;
    let Some(cleanup) = cleanup_mut(&mut target.content) else {
        return Err(EditError::Invalid("cleanup needs an audio or video item"));
    };
    let slot = cleanup
        .effects
        .iter_mut()
        .find(|other| other.id == effect.id)
        .ok_or(EditError::Invalid("cleanup effect"))?;
    *slot = effect;
    cleanup.validate()?;

    Ok(data)
}
pub(super) fn execute_remove_cleanup_effect(
    mut data: ProjectData,
    id: ItemId,
    effect: u32,
) -> Result<ProjectData> {
    let target = item(&mut data, id)?;
    let Some(cleanup) = cleanup_mut(&mut target.content) else {
        return Err(EditError::Invalid("cleanup needs an audio or video item"));
    };
    let before = cleanup.effects.len();
    cleanup.effects.retain(|other| other.id != effect);
    if cleanup.effects.len() == before {
        return Err(EditError::Invalid("cleanup effect"));
    }

    Ok(data)
}
