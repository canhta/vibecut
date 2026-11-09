//! Applies canvas size, frame rate and background commits.
use crate::editor::properties::{Editor, Result};

impl Editor {
    pub(in crate::editor::properties) fn set_canvas(
        &mut self,
        width: u32,
        height: u32,
        preset: Option<project::AspectPreset>,
    ) -> Result<()> {
        self.desk().edit_result(
            "project.set_canvas",
            serde_json::json!({
                "width": width,
                "height": height,
                "preset": preset,
            }),
        )?;
        Ok(())
    }
    pub(in crate::editor::properties) fn set_canvas_fps(
        &mut self,
        num: i64,
        den: i64,
    ) -> Result<()> {
        self.desk().edit_result(
            "project.set_fps",
            serde_json::json!({"num": num, "den": den}),
        )?;
        Ok(())
    }
    pub(in crate::editor::properties) fn set_canvas_background(
        &mut self,
        color: project::Color,
    ) -> Result<()> {
        self.desk().edit_result(
            "project.set_background",
            serde_json::json!({"color": color}),
        )?;
        Ok(())
    }
}
