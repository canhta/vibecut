//! Writes the crop channels an auto-reframe analysis owns.
use super::{Result, item};
use crate::{EditError, analysis::ReframeKeys};
use project::{
    AnimationPath, BuiltinPath, Channel, Crop, Extrapolation, Item, ItemId, Keyframe, Param,
    ParamValue, ProjectData,
};
use time::Tick;

fn channel(item: &Item, path: BuiltinPath) -> Option<Channel> {
    let param = item.animations.get(&AnimationPath::from(path))?;
    param.channel(None).ok().cloned()
}

/// Marks the shot containing `at` as manual by clearing its `crop.auto` flag.
/// Position keys stay exactly as they are; the user edits them with the ordinary
/// keyframe commands.
pub(super) fn execute_set_manual(
    mut data: ProjectData,
    id: ItemId,
    at: Tick,
) -> Result<ProjectData> {
    let target = item(&mut data, id)?;
    let auto = channel(target, BuiltinPath::CropAuto)
        .ok_or(EditError::Invalid("reframe is not generated"))?;
    if at < Tick::ZERO || at >= target.duration()? {
        return Err(EditError::Invalid("reframe shot time"));
    }
    let start = auto
        .keys()
        .iter()
        .rev()
        .find(|key| key.time <= at)
        .map(|key| key.time)
        .ok_or(EditError::Invalid("reframe shot time"))?;
    let mut keys = auto.keys().to_vec();
    let index = keys
        .binary_search_by_key(&start, |key| key.time)
        .map_err(|_| EditError::Invalid("reframe shot"))?;
    let Some(key) = keys.get_mut(index) else {
        return Err(EditError::Invalid("reframe shot"));
    };
    if key.value == ParamValue::Bool(false) {
        return Ok(data);
    }
    key.value = ParamValue::Bool(false);
    let mut auto = auto;
    auto.put(
        keys.get(index)
            .cloned()
            .ok_or(EditError::Invalid("reframe shot"))?,
    )?;
    target.animations.set(
        AnimationPath::from(BuiltinPath::CropAuto),
        Param::Value(auto),
    )?;
    Ok(data)
}

/// Removes the three crop channels and leaves the static crop untouched.
pub(super) fn execute_clear(mut data: ProjectData, id: ItemId) -> Result<ProjectData> {
    let target = item(&mut data, id)?;
    for path in [
        BuiltinPath::CropX,
        BuiltinPath::CropY,
        BuiltinPath::CropAuto,
    ] {
        target.animations.remove(&AnimationPath::from(path));
    }
    Ok(data)
}

/// Merges one item's emitted keys with its current channels. A shot whose
/// `crop.auto` value at its generated start is false is manual and keeps its
/// position keys; `replace_manual` regenerates every shot. The two items of one
/// source span still emit their own keys from the same cached analysis.
pub(crate) fn merge(
    item: &Item,
    keys: ReframeKeys,
    replace_manual: bool,
) -> Result<(Crop, Channel, Channel, Channel)> {
    let duration = item.duration()?;
    let auto = channel(item, BuiltinPath::CropAuto);
    let starts: Vec<Tick> = keys.shots.iter().map(|shot| shot.start).collect();
    let manual: Vec<bool> = starts
        .iter()
        .map(|start| {
            !replace_manual
                && auto
                    .as_ref()
                    .is_some_and(|auto| auto.value_at(*start) == ParamValue::Bool(false))
        })
        .collect();
    let x = merged_channel(
        channel(item, BuiltinPath::CropX),
        &keys.x,
        &starts,
        &manual,
        duration,
    )?;
    let y = merged_channel(
        channel(item, BuiltinPath::CropY),
        &keys.y,
        &starts,
        &manual,
        duration,
    )?;
    let auto = Channel::new(
        ParamValue::Bool(true),
        starts
            .iter()
            .zip(&manual)
            .map(|(start, manual)| {
                let mut key = Keyframe::new(*start, ParamValue::Bool(!*manual));
                key.segment = project::Segment::Step;
                key
            })
            .collect(),
        Extrapolation::Hold,
        Extrapolation::Hold,
    )?;
    Ok((keys.crop, x, y, auto))
}

/// Keeps the existing keys of manual shots and replaces every regenerated
/// shot's span with the emitted keys.
fn merged_channel(
    existing: Option<Channel>,
    generated: &Channel,
    starts: &[Tick],
    manual: &[bool],
    duration: Tick,
) -> Result<Channel> {
    let base = existing.as_ref().map_or_else(
        || generated.base().clone(),
        |existing| existing.base().clone(),
    );
    let mut keys = existing.map_or_else(Vec::new, |channel| channel.keys().to_vec());
    for (index, start) in starts.iter().enumerate() {
        if manual.get(index).copied().unwrap_or(false) {
            continue;
        }
        let end = starts.get(index + 1).copied().unwrap_or(duration);
        keys.retain(|key| key.time < *start || key.time >= end);
        keys.extend(
            generated
                .keys()
                .iter()
                .filter(|key| key.time >= *start && key.time < end)
                .cloned(),
        );
    }
    keys.sort_by_key(|key| key.time);
    Ok(Channel::new(
        base,
        keys,
        Extrapolation::Hold,
        Extrapolation::Hold,
    )?)
}
