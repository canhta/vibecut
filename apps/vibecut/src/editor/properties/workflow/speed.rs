//! Applies retiming requests.

use crate::editor::properties::{Editor, ItemId, ItemParams, Result};

impl Editor {
    /// Source and retimed duration in seconds, for display only.
    pub(in crate::editor) fn retime_seconds(&self, item: ItemId) -> Option<(f64, f64)> {
        let found = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)?;
        Some((
            found.source_range.duration().seconds(),
            found.duration().ok()?.seconds(),
        ))
    }
    /// Speed, pitch policy and whether the item has audio to retime.
    pub(in crate::editor) fn retime_of(&self, item: ItemId) -> Option<(f64, bool, bool)> {
        let found = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)?;
        let audible = match &found.content {
            ItemParams::Audio(_) => true,
            ItemParams::Video(params) => params.source_audio_enabled,
            _ => return None,
        };
        Some((
            found.retime.speed.as_f64(),
            found.retime.maintain_pitch,
            audible,
        ))
    }
    /// One element's committed speed edit through the registered command.
    pub fn set_speed(&mut self, item: ItemId, speed: f64, maintain_pitch: bool) -> Result<()> {
        self.session.set_selection(self.selection.clone())?;
        self.session.execute(
            "timeline.set_speed",
            serde_json::json!({"items":[item],"speed":speed,"maintain_pitch":maintain_pitch}),
        )?;
        self.sync_preview();
        Ok(())
    }
}
