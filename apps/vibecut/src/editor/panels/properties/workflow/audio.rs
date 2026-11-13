//! Applies the Audio tab's fade edits through `audio.set_fade`.
use super::super::read;
use crate::editor::{Desk, Error, ItemId, Result};

/// One fade edge in seconds. The engine clamps it to the clip and the other
/// fade and keeps the curve the edge already has.
pub(super) fn set_fade(
    desk: &mut Desk<'_>,
    item: ItemId,
    edge: project::FadeEdge,
    seconds: f64,
) -> Result<()> {
    let curve = read::fade_of(desk.view().session.project(), item, edge)
        .ok_or(Error::Invalid("fade needs an audio or video item"))?
        .curve;
    let duration = time::Tick::from_seconds_f64(seconds, time::Rounding::Nearest)?;
    desk.apply_result(engine::Command::SetFade {
        item,
        edge,
        duration,
        curve,
    })?;
    Ok(())
}

/// The Noise reduction switch. Turning it on enables the item's noise reduction effect, or adds
/// one with the default strength when it has none; turning it off disables the effect and keeps
/// its values. One command, so one undo step.
pub(super) fn set_noise_reduction(desk: &mut Desk<'_>, item: ItemId, enabled: bool) -> Result<()> {
    let project = desk.view().session.project();
    let command = match read::noise_reduction(project, item) {
        Some(effect) if effect.enabled == enabled => return Ok(()),
        Some(effect) => engine::Command::UpdateCleanupEffect {
            item,
            effect: project::CleanupEffect {
                enabled,
                ..effect.clone()
            },
        },
        None if enabled => engine::Command::AddCleanupEffect {
            item,
            effect: project::CleanupEffect {
                id: read::next_cleanup_effect_id(project, item).ok_or(Error::Invalid(
                    "noise reduction needs an audio or video item",
                ))?,
                enabled: true,
                wet_percent: 100.0,
                kind: project::NOISE_REDUCTION_DEFAULT,
            },
        },
        None => return Ok(()),
    };
    desk.apply_result(command)?;
    Ok(())
}
