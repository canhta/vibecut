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
