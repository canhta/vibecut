//! Applies the Audio tab's fade edits through `audio.set_fade`.
use super::super::read;
use crate::editor::{Error, ItemId, Port, Result};

/// One fade edge in seconds. The engine clamps it to the clip and the other
/// fade and keeps the curve the edge already has.
pub(super) fn set_fade(
    port: &mut Port<'_>,
    item: ItemId,
    edge: project::FadeEdge,
    seconds: f64,
) -> Result<()> {
    let curve = read::fade_of(port.view().session.project(), item, edge)
        .ok_or(Error::Invalid("fade needs an audio or video item"))?
        .curve;
    let duration = time::Tick::from_seconds_f64(seconds, time::Rounding::Nearest)?;
    port.apply_result(engine::Command::SetFade {
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
pub(super) fn set_noise_reduction(port: &mut Port<'_>, item: ItemId, enabled: bool) -> Result<()> {
    let project = port.view().session.project();
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
    port.apply_result(command)?;
    Ok(())
}

/// The Voice enhancement preset over the item's cleanup chain. `enabled` adds the preset's
/// effects (when missing and turning on) or sets every present one on or off; `wet_percent`
/// sets the mix of every present or added one. Values the user tuned stay. All edits go in one
/// batch, so one undo step, and nothing changes when nothing would.
pub(super) fn set_voice_enhancement(
    port: &mut Port<'_>,
    item: ItemId,
    enabled: Option<bool>,
    wet_percent: Option<f32>,
) -> Result<()> {
    let project = port.view().session.project();
    let present = read::cleanup_effects(project, item).ok_or(Error::Invalid(
        "voice enhancement needs an audio or video item",
    ))?;
    let mut commands = Vec::new();
    for want in project::voice_enhancement_effects() {
        match present.iter().find(|effect| effect.id == want.id) {
            Some(effect) => {
                let changed = project::CleanupEffect {
                    enabled: enabled.unwrap_or(effect.enabled),
                    wet_percent: wet_percent.unwrap_or(effect.wet_percent),
                    ..effect.clone()
                };
                if changed != *effect {
                    commands.push(engine::Command::UpdateCleanupEffect {
                        item,
                        effect: changed,
                    });
                }
            }
            None if enabled == Some(true) => {
                commands.push(engine::Command::AddCleanupEffect {
                    item,
                    effect: project::CleanupEffect {
                        wet_percent: wet_percent.unwrap_or(want.wet_percent),
                        ..want
                    },
                });
            }
            None => {}
        }
    }
    let command = match commands.len() {
        0 => return Ok(()),
        1 => commands.remove(0),
        _ => engine::Command::Batch { commands },
    };
    port.apply_result(command)?;
    Ok(())
}
