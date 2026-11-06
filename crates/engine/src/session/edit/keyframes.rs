//! Applies keyframes commands to a candidate project.
use super::{ItemId, ProjectData, Result, Tick, item};
pub(super) fn execute_set_animated_param(
    mut data: ProjectData,
    id: ItemId,
    path: project::AnimationPath,
    param: project::Param,
) -> Result<ProjectData> {
    item(&mut data, id)?.animations.set(path, param)?;

    Ok(data)
}
/// Add a key at `time`, or remove the one already there. `value` is used for a
/// missing channel or missing key; the caller reads the field's current value.
pub(super) fn execute_toggle_keyframe(
    mut data: ProjectData,
    id: ItemId,
    path: project::AnimationPath,
    component: Option<project::ColorComponent>,
    time: Tick,
    value: project::ParamValue,
) -> Result<ProjectData> {
    let target = item(&mut data, id)?;
    let mut param = match target.animations.get(&path) {
        Some(param) => param.clone(),
        None => project::Param::new(value.clone())?,
    };
    let channel = param.channel_mut(component)?;
    if channel
        .keys()
        .binary_search_by_key(&time, |key| key.time)
        .is_ok()
    {
        channel.remove(time)?;
    } else {
        channel.put(project::Keyframe::new(time, value))?;
    }
    target.animations.set(path, param)?;
    Ok(data)
}
/// Remove several keyframes atomically; a stale entry refuses every removal.
pub(super) fn execute_delete_keyframes(
    mut data: ProjectData,
    keys: Vec<crate::KeyframeRef>,
) -> Result<ProjectData> {
    if keys.is_empty() {
        return Err(crate::EditError::Invalid("empty keyframe selection"));
    }
    for key in keys {
        item(&mut data, key.item)?
            .animations
            .get_mut(&key.path)?
            .channel_mut(key.component)?
            .remove(key.time)?;
    }
    Ok(data)
}
pub(super) fn execute_set_keyframe_extrapolation(
    mut data: ProjectData,
    id: ItemId,
    path: project::AnimationPath,
    component: Option<project::ColorComponent>,
    before: project::Extrapolation,
    after: project::Extrapolation,
) -> Result<ProjectData> {
    item(&mut data, id)?
        .animations
        .get_mut(&path)?
        .channel_mut(component)?
        .set_extrapolation(before, after)?;
    Ok(data)
}
pub(super) fn execute_add_keyframe(
    mut data: ProjectData,
    id: ItemId,
    path: project::AnimationPath,
    component: Option<project::ColorComponent>,
    key: project::Keyframe,
) -> Result<ProjectData> {
    item(&mut data, id)?
        .animations
        .get_mut(&path)?
        .channel_mut(component)?
        .put(key)?;

    Ok(data)
}
pub(super) fn execute_remove_keyframe(
    mut data: ProjectData,
    id: ItemId,
    path: project::AnimationPath,
    component: Option<project::ColorComponent>,
    time: Tick,
) -> Result<ProjectData> {
    item(&mut data, id)?
        .animations
        .get_mut(&path)?
        .channel_mut(component)?
        .remove(time)?;

    Ok(data)
}
pub(super) fn execute_retime_keyframe(
    mut data: ProjectData,
    id: ItemId,
    path: project::AnimationPath,
    component: Option<project::ColorComponent>,
    from: Tick,
    to: Tick,
) -> Result<ProjectData> {
    item(&mut data, id)?
        .animations
        .get_mut(&path)?
        .channel_mut(component)?
        .retime_key(from, to)?;

    Ok(data)
}
#[allow(clippy::too_many_arguments)]
pub(super) fn execute_set_keyframe_handle(
    mut data: ProjectData,
    id: ItemId,
    path: project::AnimationPath,
    component: Option<project::ColorComponent>,
    time: Tick,
    side: project::HandleSide,
    handle: Option<project::CurveHandle>,
    tangent: project::TangentMode,
) -> Result<ProjectData> {
    item(&mut data, id)?
        .animations
        .get_mut(&path)?
        .channel_mut(component)?
        .set_handle(time, side, handle, tangent)?;

    Ok(data)
}
pub(super) fn execute_scale_keyframe_time(
    mut data: ProjectData,
    id: ItemId,
    numerator: i64,
    denominator: i64,
) -> Result<ProjectData> {
    item(&mut data, id)?
        .animations
        .retime(numerator, denominator)?;

    Ok(data)
}
