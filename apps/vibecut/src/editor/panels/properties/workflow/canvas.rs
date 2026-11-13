//! Applies canvas size and frame rate commits.
use crate::editor::{Port, Result};

pub(super) fn set_canvas(
    port: &mut Port<'_>,
    width: u32,
    height: u32,
    preset: Option<project::AspectPreset>,
) -> Result<()> {
    port.apply_result(engine::Command::SetCanvas {
        width,
        height,
        preset,
    })?;
    Ok(())
}
pub(super) fn set_canvas_fps(port: &mut Port<'_>, num: i64, den: i64) -> Result<()> {
    port.apply_result(engine::Command::SetFps { num, den })?;
    Ok(())
}
