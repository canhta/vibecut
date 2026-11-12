//! Aligns and evenly spaces selections by translating transform position.
use super::{EditError, ItemId, ProjectData, Result, item};
use project::{
    AnimationPath, Channel, DistributionAxis, ItemAlignment, Param, ParamValue, TransformField,
};
use std::collections::BTreeSet;
use time::Tick;

/// The smallest translation treated as a real change, in project pixels.
const RESIDUE: f64 = 1e-9;

/// Move every listed item to one canvas edge or centre. Each item is aligned
/// against the canvas independently, not against the selection envelope.
pub(super) fn execute_align(
    mut data: ProjectData,
    items: Vec<ItemId>,
    alignment: ItemAlignment,
    at: Tick,
) -> Result<ProjectData> {
    let targets = targets(&data, &items, at)?;
    let width = f64::from(data.canvas.width);
    let height = f64::from(data.canvas.height);
    for (id, bounds) in targets {
        let (dx, dy) = match alignment {
            ItemAlignment::Left => (-bounds.x, 0.0),
            ItemAlignment::HorizontalCenter => (width / 2.0 - (bounds.x + bounds.width / 2.0), 0.0),
            ItemAlignment::Right => (width - (bounds.x + bounds.width), 0.0),
            ItemAlignment::Top => (0.0, -bounds.y),
            ItemAlignment::VerticalCenter => (0.0, height / 2.0 - (bounds.y + bounds.height / 2.0)),
            ItemAlignment::Bottom => (0.0, height - (bounds.y + bounds.height)),
        };
        if dx.abs() > RESIDUE {
            translate(&mut data, id, TransformField::PositionX, dx)?;
        }
        if dy.abs() > RESIDUE {
            translate(&mut data, id, TransformField::PositionY, dy)?;
        }
    }
    Ok(data)
}

/// Space three or more listed items evenly along one axis, pinning the first
/// leading edge and the last trailing edge. The gaps between boxes are equal.
pub(super) fn execute_distribute(
    mut data: ProjectData,
    items: Vec<ItemId>,
    axis: DistributionAxis,
    at: Tick,
) -> Result<ProjectData> {
    let targets = targets(&data, &items, at)?;
    if targets.len() < 3 {
        return Err(EditError::Invalid("distribute needs three items"));
    }
    // Bounds are plain geometry; the item ID breaks every exact tie, so input
    // order never decides which item lands where.
    let horizontal = axis == DistributionAxis::Horizontal;
    let mut sorted = targets
        .into_iter()
        .map(|(id, bounds)| {
            let (leading, extent) = if horizontal {
                (bounds.x, bounds.width)
            } else {
                (bounds.y, bounds.height)
            };
            (id, leading, extent)
        })
        .collect::<Vec<_>>();
    sorted.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    let first = sorted[0].1;
    let last = sorted
        .last()
        .map(|target| target.1 + target.2)
        .unwrap_or(first);
    let total: f64 = sorted.iter().map(|target| target.2).sum();
    let gap = (last - first - total) / (sorted.len() - 1) as f64;
    if !gap.is_finite() || gap < -RESIDUE {
        return Err(EditError::Invalid("items overlap for distribution"));
    }
    let gap = gap.max(0.0);
    let field = if horizontal {
        TransformField::PositionX
    } else {
        TransformField::PositionY
    };
    let mut cursor = first;
    for (index, (id, leading, extent)) in sorted.iter().enumerate() {
        if index > 0 {
            let delta = cursor - *leading;
            if delta.abs() > RESIDUE {
                translate(&mut data, *id, field, delta)?;
            }
        }
        cursor += *extent + gap;
    }
    Ok(data)
}

/// Resolve the whole explicit selection to its canvas envelopes before any
/// write. Missing, other-scene, duplicate, unsupported or locked targets
/// refuse the operation without a partial change.
fn targets(data: &ProjectData, items: &[ItemId], at: Tick) -> Result<Vec<(ItemId, render::Rect)>> {
    if items.is_empty() {
        return Err(EditError::Invalid("empty item selection"));
    }
    if at < Tick::ZERO {
        return Err(EditError::Invalid("negative arrangement time"));
    }
    let mut remaining: BTreeSet<_> = items.iter().copied().collect();
    if remaining.len() != items.len() {
        return Err(EditError::Invalid("duplicate item selection"));
    }
    let mut targets = Vec::with_capacity(items.len());
    for track in data.scene().timeline.iter() {
        for target in &track.items {
            if !remaining.remove(&target.id) {
                continue;
            }
            if track.locked {
                return Err(EditError::Locked(track.id));
            }
            if !target.content.supports_arrange() {
                return Err(EditError::Invalid("arrange on unsupported item"));
            }
            let bounds = render::arrange::bounds(data, target, at)
                .map_err(|_| EditError::Invalid("item arrangement geometry"))?;
            targets.push((target.id, bounds));
        }
    }
    if let Some(id) = remaining.into_iter().next() {
        return Err(EditError::NoItem(id));
    }
    Ok(targets)
}

/// Translate one position channel by `delta`, moving its base and every scalar
/// key. Key times, segments, handles and extrapolation are preserved. The raw
/// and shifted values must both fit the channel's admitted numeric range, so
/// evaluation clamping can never shorten the requested move.
fn translate(data: &mut ProjectData, id: ItemId, field: TransformField, delta: f64) -> Result<()> {
    let path = AnimationPath::from(field.builtin());
    let target = item(data, id)?;
    let existing = target.animations.get(&path).cloned();
    let replaced = match existing {
        None => {
            let base = shifted(field.default_value(), delta, field)?;
            Param::new(ParamValue::Scalar(base))?
        }
        Some(param) => {
            let channel = param.channel(None)?;
            let ParamValue::Scalar(base) = channel.base() else {
                return Err(EditError::Invalid("transform scalar"));
            };
            let base = shifted(*base, delta, field)?;
            let mut keys = Vec::with_capacity(channel.keys().len());
            for key in channel.keys() {
                let ParamValue::Scalar(value) = &key.value else {
                    return Err(EditError::Invalid("transform scalar"));
                };
                let mut key = key.clone();
                key.value = ParamValue::Scalar(shifted(*value, delta, field)?);
                keys.push(key);
            }
            let (before, after) = channel.extrapolation();
            Param::Value(Channel::new(ParamValue::Scalar(base), keys, before, after)?)
        }
    };
    target.animations.set(path, replaced)?;
    Ok(())
}

/// Reject values outside the transform field's admitted range instead of
/// clamping them, so a refused move never partially applies.
fn shifted(value: f64, delta: f64, field: TransformField) -> Result<f64> {
    let (min, max) = field.bounds();
    let shifted = value + delta;
    if !value.is_finite()
        || !shifted.is_finite()
        || !(min..=max).contains(&value)
        || !(min..=max).contains(&shifted)
    {
        return Err(EditError::Invalid("alignment position bound"));
    }
    Ok(shifted)
}
