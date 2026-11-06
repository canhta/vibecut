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
