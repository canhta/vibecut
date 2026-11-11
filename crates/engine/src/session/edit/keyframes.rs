//! Applies keyframes commands to a candidate project.
use super::{ItemId, ProjectData, Result, Tick, item};

impl crate::TextAnimationPreset {
    /// Exact saved-Param match; custom curves and invalid/short items have no selected choice.
    pub fn matches(self, target: &project::Item, fps: time::FrameRate) -> bool {
        self.opacity(target, fps).is_ok_and(|expected| {
            target.animations.get(&project::BuiltinPath::Opacity.into()) == Some(&expected)
        })
    }

    fn opacity(self, target: &project::Item, fps: time::FrameRate) -> Result<project::Param> {
        use project::{Channel, Extrapolation::Hold, Keyframe, Param, ParamValue::Scalar};
        if !matches!(target.content, project::ItemParams::Text(_)) {
            return Err(crate::session::EditError::Invalid(
                "text animation requires text",
            ));
        }
        let length = target.duration()?;
        let minimum = fps
            .tick_of(1)?
            .checked_add(fps.tick_of(1)?)
            .ok_or(crate::session::EditError::Invalid("time overflow"))?;
        if length < minimum {
            return Err(crate::session::EditError::Invalid(
                "text animation requires two frames",
            ));
        }
        let base = match target.animations.get(&project::BuiltinPath::Opacity.into()) {
            None => 1.0,
            Some(Param::Value(channel)) => match channel.base() {
                Scalar(base) => *base,
                _ => return Err(crate::session::EditError::Invalid("text opacity base")),
            },
            _ => return Err(crate::session::EditError::Invalid("text opacity base")),
        };
        if !base.is_finite() || !(0.0..=1.0).contains(&base) {
            return Err(crate::session::EditError::Invalid("text opacity base"));
        }
        let d = Tick(76_204_800_000.min(length.0 / 2));
        let exit = length
            .checked_sub(d)
            .ok_or(crate::session::EditError::Invalid("time overflow"))?;
        let pairs = match self {
            Self::FadeIn => vec![(Tick::ZERO, 0.0), (d, base)],
            Self::FadeOut => vec![(exit, base), (length, 0.0)],
            Self::FadeInOut if d == exit => vec![(Tick::ZERO, 0.0), (d, base), (length, 0.0)],
            Self::FadeInOut => vec![(Tick::ZERO, 0.0), (d, base), (exit, base), (length, 0.0)],
        };
        Ok(Param::Value(Channel::new(
            Scalar(base),
            pairs
                .into_iter()
                .map(|(time, value)| Keyframe::new(time, Scalar(value)))
                .collect(),
            Hold,
            Hold,
        )?))
    }
}

pub(super) fn execute_apply_text_animation(
    mut data: ProjectData,
    id: ItemId,
    preset: crate::TextAnimationPreset,
) -> Result<ProjectData> {
    let fps = data.canvas.fps;
    let target = item(&mut data, id)?; // Locks also refuse exact repeats.
    let expected = preset.opacity(target, fps)?;
    let path = project::BuiltinPath::Opacity.into();
    if let Some(existing) = target.animations.get(&path) {
        if existing == &expected {
            return Ok(data);
        }
        if existing.channel(None)?.keys().len() != 0 {
            return Err(crate::session::EditError::Invalid(
                "remove opacity keys before changing animation",
            ));
        }
    }
    target.animations.set(path, expected)?;
    Ok(data)
}
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
        return Err(crate::session::EditError::Invalid(
            "empty keyframe selection",
        ));
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
