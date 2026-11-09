//! Applies retiming requests.

use crate::editor::properties::{Editor, ItemId, Result};

impl Editor {
    /// One element's committed speed edit through the registered command.
    pub fn set_speed(&mut self, item: ItemId, speed: f64, maintain_pitch: bool) -> Result<()> {
        self.desk().edit_result(
            "timeline.set_speed",
            serde_json::json!({"items":[item],"speed":speed,"maintain_pitch":maintain_pitch}),
        )?;
        Ok(())
    }
}
