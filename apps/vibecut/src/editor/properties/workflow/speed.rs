//! Applies retiming requests.

use crate::editor::properties::{Editor, ItemId, Result};

impl Editor {
    /// One element's committed speed edit through the registered command.
    pub fn set_speed(&mut self, item: ItemId, speed: f64, maintain_pitch: bool) -> Result<()> {
        self.desk().apply_result(
            engine::Command::SetSpeed {
                items: vec![item],
                speed: time::Speed::from_f64(speed)?,
                maintain_pitch,
            },
            false,
        )?;
        Ok(())
    }
}
