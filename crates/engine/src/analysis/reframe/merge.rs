//! Merges emitted reframe keys with an item's current crop channels.
use super::ReframeKeys;
use crate::EditError;
use project::{
    AnimationPath, BuiltinPath, Channel, Crop, Extrapolation, Item, Keyframe, ParamValue,
};
use time::Tick;

type Result<T> = std::result::Result<T, EditError>;

fn channel(item: &Item, path: BuiltinPath) -> Option<Channel> {
    let param = item.animations.get(&AnimationPath::from(path))?;
    param.channel(None).ok().cloned()
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
