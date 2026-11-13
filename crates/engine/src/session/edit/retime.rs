//! Applies retime commands to a candidate project.
use super::{BTreeSet, EditError, ItemId, ItemParams, ProjectData, Result, add, sub, unlocked};

/// Rewrite one video/audio item's retime through `change`, which returns the
/// factor `(num, den)` its keyframes stretch by (new duration over old). On a
/// magnetic main track later items ripple by the duration change.
fn retime_item(
    data: &mut ProjectData,
    id: ItemId,
    unsupported: &'static str,
    change: impl FnOnce(&mut project::Item) -> Result<(i64, i64)>,
) -> Result<()> {
    let timeline = &mut data.scene_mut().timeline;
    let ripple = timeline.magnet && timeline.main.items.iter().any(|i| i.id == id);
    let Some(t) = timeline
        .iter_mut()
        .find(|t| t.items.iter().any(|i| i.id == id))
    else {
        return Err(EditError::NoItem(id));
    };
    unlocked(t)?;
    let index = t
        .items
        .iter()
        .position(|i| i.id == id)
        .ok_or(EditError::NoItem(id))?;
    let target = t.items.get_mut(index).ok_or(EditError::NoItem(id))?;
    if !matches!(target.content, ItemParams::Video(_) | ItemParams::Audio(_)) {
        return Err(EditError::Unsupported(unsupported));
    }
    let old_end = target.end()?;
    let (num, den) = change(target)?;
    if num != den {
        target.animations.retime(num, den)?;
    }
    let new_end = target.end()?;
    if ripple {
        let delta = sub(new_end, old_end)?;
        for later in t.items.iter_mut().skip(index + 1) {
            if later.start >= old_end {
                later.start = add(later.start, delta)?;
            }
        }
    }
    Ok(())
}

/// Change one video/audio item's speed, keeping its source window. Keyframes stretch
/// with the item; on a magnetic main track later items ripple by the duration change.
/// A speed curve on the item is replaced by the constant rate.
pub(super) fn set_speed(
    data: &mut ProjectData,
    id: ItemId,
    speed: time::Speed,
    maintain_pitch: bool,
) -> Result<()> {
    retime_item(data, id, "speed on non-media item", |target| {
        let had_curve = target.retime.curve.is_some();
        let old_duration = target.duration()?;
        let ratio = target.retime.speed.stretch_to(speed);
        let keep_window = !had_curve && target.retime.speed == speed;
        target.retime = project::Retime {
            window: if keep_window {
                target.retime.window
            } else {
                None
            },
            speed,
            maintain_pitch,
            reverse: target.retime.reverse,
            curve: None,
        };
        if had_curve {
            Ok((target.duration()?.0, old_duration.0))
        } else {
            Ok(ratio)
        }
    })
}

/// Give one video/audio item a whole `Retime` value through the same key stretch and
/// ripple as a speed edit. A curve is rebuilt over this item's visible source span
/// from its stretches; a retained window survives only while the constant speed is
/// unchanged and neither side has a curve.
pub(super) fn set_retime(
    data: &mut ProjectData,
    id: ItemId,
    retime: project::Retime,
) -> Result<()> {
    retime_item(data, id, "retime on non-media item", |target| {
        let old = target.retime;
        let old_duration = target.duration()?;
        let curve = match retime.curve {
            Some(_) if retime.reverse => {
                return Err(EditError::Invalid("speed curve on a reversed clip"));
            }
            Some(curve) => {
                // A curve cannot be reversed, so the direction flips first.
                target.set_reversed(false)?;
                Some(project::SpeedCurve::from_segments(
                    target.source_range.duration(),
                    &curve.segments(),
                )?)
            }
            None => None,
        };
        let curved = old.curve.is_some() || curve.is_some();
        let keep_window = !curved && old.speed == retime.speed;
        target.retime = project::Retime {
            window: if keep_window {
                target.retime.window
            } else {
                None
            },
            reverse: target.retime.reverse,
            curve,
            ..retime
        };
        // A direction flip mirrors a retained window, so it goes through the item's own rule.
        target.set_reversed(retime.reverse)?;
        if curved {
            Ok((target.duration()?.0, old_duration.0))
        } else {
            Ok(old.speed.stretch_to(retime.speed))
        }
    })
}

/// Give one video/audio item a speed curve over its visible source range,
/// replacing its constant rate. Keyframes stretch with the item.
fn set_speed_curve(
    data: &mut ProjectData,
    id: ItemId,
    segments: &[project::SpeedSegment],
    maintain_pitch: bool,
) -> Result<()> {
    retime_item(data, id, "speed curve on non-media item", |target| {
        if target.retime.reverse {
            return Err(EditError::Invalid("speed curve on a reversed clip"));
        }
        let old_duration = target.duration()?;
        let curve = project::SpeedCurve::from_segments(target.source_range.duration(), segments)?;
        target.retime = project::Retime {
            window: None,
            speed: time::Speed::UNITY,
            maintain_pitch,
            reverse: false,
            curve: Some(curve),
        };
        Ok((target.duration()?.0, old_duration.0))
    })
}

pub(super) fn execute_set_speed_curve(
    mut data: ProjectData,
    items: Vec<ItemId>,
    segments: Vec<project::SpeedSegment>,
    maintain_pitch: bool,
) -> Result<ProjectData> {
    if items.is_empty() {
        return Err(EditError::Invalid("empty item selection"));
    }
    let mut seen = BTreeSet::new();
    for id in items {
        if !seen.insert(id) {
            return Err(EditError::Invalid("duplicate speed item"));
        }
        set_speed_curve(&mut data, id, &segments, maintain_pitch)?;
    }
    Ok(data)
}

pub(super) fn execute_apply_speed_preset(
    data: ProjectData,
    items: Vec<ItemId>,
    preset: project::SpeedPreset,
    maintain_pitch: bool,
) -> Result<ProjectData> {
    execute_set_speed_curve(data, items, preset.segments(), maintain_pitch)
}

/// Flip one video/audio item's traversal direction. Its timeline span is kept:
/// the item plays the same source span backwards, including a split or trimmed
/// piece, whose retained window is mirrored (see `Item::set_reversed`).
fn set_reverse(data: &mut ProjectData, id: ItemId, reverse: bool) -> Result<()> {
    let timeline = &mut data.scene_mut().timeline;
    let Some(t) = timeline
        .iter_mut()
        .find(|t| t.items.iter().any(|i| i.id == id))
    else {
        return Err(EditError::NoItem(id));
    };
    unlocked(t)?;
    let target = t
        .items
        .iter_mut()
        .find(|i| i.id == id)
        .ok_or(EditError::NoItem(id))?;
    if !matches!(target.content, ItemParams::Video(_) | ItemParams::Audio(_)) {
        return Err(EditError::Unsupported("reverse on non-media item"));
    }
    target.set_reversed(reverse)?;
    Ok(())
}

pub(super) fn execute_set_reverse(
    mut data: ProjectData,
    items: Vec<ItemId>,
    reverse: bool,
) -> Result<ProjectData> {
    if items.is_empty() {
        return Err(EditError::Invalid("empty item selection"));
    }
    let mut seen = BTreeSet::new();
    for id in items {
        if !seen.insert(id) {
            return Err(EditError::Invalid("duplicate reverse item"));
        }
        set_reverse(&mut data, id, reverse)?;
    }
    Ok(data)
}

pub(super) fn execute_set_speed(
    mut data: ProjectData,
    items: Vec<ItemId>,
    speed: time::Speed,
    maintain_pitch: bool,
) -> Result<ProjectData> {
    if items.is_empty() {
        return Err(EditError::Invalid("empty item selection"));
    }
    let mut seen = BTreeSet::new();
    for id in items {
        if !seen.insert(id) {
            return Err(EditError::Invalid("duplicate speed item"));
        }
        set_speed(&mut data, id, speed, maintain_pitch)?;
    }

    Ok(data)
}
