//! Applies canvas size, frame rate and background commits.
use crate::editor::{Desk, Result};

pub(super) fn set_canvas(
    desk: &mut Desk<'_>,
    width: u32,
    height: u32,
    preset: Option<project::AspectPreset>,
) -> Result<()> {
    desk.apply_result(
        engine::Command::SetCanvas {
            width,
            height,
            preset,
        },
        false,
    )?;
    Ok(())
}
pub(super) fn set_canvas_fps(desk: &mut Desk<'_>, num: i64, den: i64) -> Result<()> {
    desk.apply_result(engine::Command::SetFps { num, den }, false)?;
    Ok(())
}
pub(super) fn set_canvas_background(desk: &mut Desk<'_>, color: project::Color) -> Result<()> {
    desk.apply_result(engine::Command::SetBackground { color }, false)?;
    Ok(())
}
