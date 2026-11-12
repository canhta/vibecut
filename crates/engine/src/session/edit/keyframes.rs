//! Applies keyframes commands to a candidate project.
use super::{ItemId, ProjectData, Result, Tick, item};
use project::{BuiltinPath, Channel, Extrapolation::Hold, Keyframe, Param, ParamValue::Scalar};

/// One animation edge, in integer ticks (300 ms at any canvas rate).
const EDGE_TICKS: i64 = 76_204_800_000;
/// Custom slide travel as a fraction of the project canvas width.
const SLIDE_FRACTION: f64 = 0.25;
/// Custom scale entrance/exit relative to the item's current scale.
const SCALE_FACTOR: f64 = 0.5;

/// Channels owned by text animation presets; every other channel is user animation.
const ANIMATION_PATHS: [BuiltinPath; 4] = [
    BuiltinPath::Opacity,
    BuiltinPath::TransformPositionX,
    BuiltinPath::TransformScaleX,
    BuiltinPath::TransformScaleY,
];

impl crate::TextAnimationPreset {
    /// Exact saved-Param match on every owned channel; a custom curve or key on
    /// another animation channel has no selected choice. Invalid/short items too.
    pub fn matches(self, target: &project::Item, data: &project::ProjectData) -> bool {
        self.expected(target, &data.canvas).is_ok_and(|expected| {
            expected
                .iter()
                .all(|(path, param)| target.animations.get(&(*path).into()) == Some(param))
                && ANIMATION_PATHS.into_iter().all(|path| {
                    expected.iter().any(|(owned, _)| *owned == path)
                        || target.animations.get(&path.into()).is_none_or(|param| {
                            param
                                .channel(None)
                                .is_ok_and(|channel| channel.keys().is_empty())
                        })
                })
        })
    }

    /// Ordinary Params this preset expands to, including the item's current bases.
    fn expected(
        self,
        target: &project::Item,
        canvas: &project::Canvas,
    ) -> Result<Vec<(BuiltinPath, Param)>> {
        if !matches!(target.content, project::ItemParams::Text(_)) {
            return Err(crate::session::EditError::Invalid(
                "text animation requires text",
            ));
        }
        let length = target.duration()?;
        let minimum = canvas
            .fps
            .tick_of(1)?
            .checked_add(canvas.fps.tick_of(1)?)
            .ok_or(crate::session::EditError::Invalid("time overflow"))?;
        if length < minimum {
            return Err(crate::session::EditError::Invalid(
                "text animation requires two frames",
            ));
        }
        let d = Tick(EDGE_TICKS.min(length.0 / 2));
        let exit = length
            .checked_sub(d)
            .ok_or(crate::session::EditError::Invalid("time overflow"))?;
        let start = match self {
            Self::FadeIn | Self::FadeOut | Self::FadeInOut => {
                let base = self.opacity_base(target)?;
                let pairs = self.edge_pairs(d, exit, length, 0.0, base, 0.0);
                vec![(BuiltinPath::Opacity, channel(base, pairs)?)]
            }
            Self::SlideIn | Self::SlideOut | Self::SlideInOut => {
                let base = scalar_base(target, BuiltinPath::TransformPositionX, 0.0)?;
                project::TransformField::PositionX.clamp(base)?;
                let distance = SLIDE_FRACTION * f64::from(canvas.width);
                let off = project::TransformField::PositionX.clamp(base - distance)?;
                let end = project::TransformField::PositionX.clamp(base + distance)?;
                let pairs = self.edge_pairs(d, exit, length, off, base, end);
                vec![(BuiltinPath::TransformPositionX, channel(base, pairs)?)]
            }
            Self::ScaleIn | Self::ScaleOut | Self::ScaleInOut => {
                let base_x = scalar_base(target, BuiltinPath::TransformScaleX, 1.0)?;
                let base_y = scalar_base(target, BuiltinPath::TransformScaleY, 1.0)?;
                let start_x = project::TransformField::ScaleX.clamp(base_x * SCALE_FACTOR)?;
                let start_y = project::TransformField::ScaleY.clamp(base_y * SCALE_FACTOR)?;
                let x = channel(
                    base_x,
                    self.edge_pairs(d, exit, length, start_x, base_x, start_x),
                )?;
                let y = channel(
                    base_y,
                    self.edge_pairs(d, exit, length, start_y, base_y, start_y),
                )?;
                vec![
                    (BuiltinPath::TransformScaleX, x),
                    (BuiltinPath::TransformScaleY, y),
                ]
            }
        };
        Ok(start)
    }

    /// Start, hold and end values for this preset's direction at the shared edge times.
    fn edge_pairs(
        self,
        d: Tick,
        exit: Tick,
        length: Tick,
        start: f64,
        base: f64,
        end: f64,
    ) -> Vec<(Tick, f64)> {
        match self {
            Self::FadeIn | Self::SlideIn | Self::ScaleIn => {
                vec![(Tick::ZERO, start), (d, base)]
            }
            Self::FadeOut | Self::SlideOut | Self::ScaleOut => {
                vec![(exit, base), (length, end)]
            }
            Self::FadeInOut | Self::SlideInOut | Self::ScaleInOut if d == exit => {
                vec![(Tick::ZERO, start), (d, base), (length, end)]
            }
            Self::FadeInOut | Self::SlideInOut | Self::ScaleInOut => {
                vec![(Tick::ZERO, start), (d, base), (exit, base), (length, end)]
            }
        }
    }

    fn opacity_base(self, target: &project::Item) -> Result<f64> {
        let base = scalar_base(target, BuiltinPath::Opacity, 1.0)?;
        if !(0.0..=1.0).contains(&base) {
            return Err(crate::session::EditError::Invalid("text opacity base"));
        }
        Ok(base)
    }
}

/// Start, hold and end values for one preset direction at the shared edge times.
fn channel(base: f64, pairs: Vec<(Tick, f64)>) -> Result<Param> {
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

/// Current scalar base of `path`, or the channel's documented default.
fn scalar_base(target: &project::Item, path: BuiltinPath, default: f64) -> Result<f64> {
    match target.animations.get(&path.into()) {
        None => Ok(default),
        Some(Param::Value(channel)) => match channel.base() {
            Scalar(base) if base.is_finite() => Ok(*base),
            _ => Err(crate::session::EditError::Invalid("text animation base")),
        },
        _ => Err(crate::session::EditError::Invalid("text animation base")),
    }
}

pub(super) fn execute_apply_text_animation(
    mut data: ProjectData,
    id: ItemId,
    preset: crate::TextAnimationPreset,
) -> Result<ProjectData> {
    let canvas = data.canvas.clone();
    let target = item(&mut data, id)?; // Locks also refuse exact repeats.
    let expected = preset.expected(target, &canvas)?;
    let mut unchanged = true;
    for path in ANIMATION_PATHS {
        let existing = target.animations.get(&path.into());
        let wanted = expected
            .iter()
            .find(|(candidate, _)| *candidate == path)
            .map(|(_, param)| param);
        match (existing, wanted) {
            (Some(existing), Some(wanted)) if existing == wanted => {}
            (None, None) => {}
            (existing, _) if existing.is_some_and(keyed) => {
                return Err(crate::session::EditError::Invalid(
                    "remove text animation keys before changing animation",
                ));
            }
            (_, Some(_)) => unchanged = false,
            (Some(_), None) => {}
        }
    }
    if unchanged {
        return Ok(data);
    }
    for (path, param) in expected {
        target.animations.set(path.into(), param)?;
    }
    Ok(data)
}

fn keyed(param: &Param) -> bool {
    param
        .channel(None)
        .is_ok_and(|channel| !channel.keys().is_empty())
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
/// Give the segment leaving every addressed key the same curve, atomically: a
/// stale key or a key with no outgoing segment refuses the whole selection.
pub(super) fn execute_set_easing(
    mut data: ProjectData,
    keys: Vec<crate::KeyframeRef>,
    curve: project::EasingCurve,
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
            .set_easing(key.time, curve)?;
    }
    Ok(data)
}
