//! Applies retime commands to a candidate project.
use super::{BTreeSet, EditError, ItemId, ItemParams, ProjectData, Result, add, sub, unlocked};

/// Change one video/audio item's speed, keeping its source window. Keyframes stretch
/// with the item; on a magnetic main track later items ripple by the duration change.
pub(super) fn set_speed(
    data: &mut ProjectData,
    id: ItemId,
    speed: time::Speed,
    maintain_pitch: bool,
) -> Result<()> {
    change_retime(data, id, |current| project::Retime {
        speed,
        maintain_pitch,
        ..current
    })
}

/// Give one video/audio item a whole `Retime` value through the same key stretch and
/// ripple as a speed edit. A retained window survives only while the speed is unchanged.
pub(super) fn set_retime(
    data: &mut ProjectData,
    id: ItemId,
    retime: project::Retime,
) -> Result<()> {
    change_retime(data, id, |_| retime)
}

fn change_retime(
    data: &mut ProjectData,
    id: ItemId,
    next: impl FnOnce(project::Retime) -> project::Retime,
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
        return Err(EditError::Unsupported("speed on non-media item"));
    }
    let old_end = target.end()?;
    let old = target.retime;
    let new = next(old);
    let (num, den) = old.speed.stretch_to(new.speed);
    if num != den {
        target.animations.retime(num, den)?;
    }
    target.retime = project::Retime {
        window: if old.speed == new.speed {
            old.window
        } else {
            None
        },
        reverse: old.reverse,
        ..new
    };
    // A direction flip mirrors a retained window, so it goes through the item's own rule.
    target.set_reversed(new.reverse)?;
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
