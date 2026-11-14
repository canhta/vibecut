//! Applies retiming requests.
use crate::editor::{ItemId, Port, Result};

/// One item's direction flip through `timeline.set_reverse`: one undo step.
pub(super) fn set_reverse(port: &mut Port<'_>, item: ItemId, reverse: bool) -> Result<()> {
    port.apply_result(engine::Command::SetReverse {
        items: vec![item],
        reverse,
    })?;
    Ok(())
}

/// One element's committed speed edit through the registered command.
pub(super) fn set_speed(
    port: &mut Port<'_>,
    item: ItemId,
    speed: f64,
    maintain_pitch: bool,
) -> Result<()> {
    port.apply_result(engine::Command::SetSpeed {
        items: vec![item],
        speed: time::Speed::from_f64(speed)?,
        maintain_pitch,
    })?;
    Ok(())
}

/// One item's whole speed curve through `timeline.set_speed_curve`: one undo step.
/// A speed outside 0.1-100x refuses before the command runs.
pub(super) fn set_speed_curve(
    port: &mut Port<'_>,
    item: ItemId,
    segments: &[(u16, f64)],
    maintain_pitch: bool,
) -> Result<()> {
    let segments = segments
        .iter()
        .map(|&(span, speed)| {
            Ok(project::SpeedSegment {
                span,
                speed: time::Speed::from_f64(speed)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    port.apply_result(engine::Command::SetSpeedCurve {
        items: vec![item],
        segments,
        maintain_pitch,
    })?;
    Ok(())
}

/// One item's ramp preset through `timeline.apply_speed_preset`: one undo step.
pub(super) fn apply_speed_preset(
    port: &mut Port<'_>,
    item: ItemId,
    preset: project::SpeedPreset,
    maintain_pitch: bool,
) -> Result<()> {
    port.apply_result(engine::Command::ApplySpeedPreset {
        items: vec![item],
        preset,
        maintain_pitch,
    })?;
    Ok(())
}
