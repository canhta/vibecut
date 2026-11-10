//! Applies canvas size, frame rate and background commits.
use crate::editor::properties::{Editor, Result};

impl Editor {
    pub(in crate::editor::properties) fn set_canvas(
        &mut self,
        width: u32,
        height: u32,
        preset: Option<project::AspectPreset>,
    ) -> Result<()> {
        self.desk().apply_result(
            engine::Command::SetCanvas {
                width,
                height,
                preset,
            },
            false,
        )?;
        Ok(())
    }
    pub(in crate::editor::properties) fn set_canvas_fps(
        &mut self,
        num: i64,
        den: i64,
    ) -> Result<()> {
        self.desk()
            .apply_result(engine::Command::SetFps { num, den }, false)?;
        Ok(())
    }
    pub(in crate::editor::properties) fn set_canvas_background(
        &mut self,
        color: project::Color,
    ) -> Result<()> {
        self.desk()
            .apply_result(engine::Command::SetBackground { color }, false)?;
        Ok(())
    }
}
