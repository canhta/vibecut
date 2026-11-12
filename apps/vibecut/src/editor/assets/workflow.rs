//! Applies the assets workflows.
mod import;
mod insertion;
mod music;
mod selection;
mod sounds;
mod transition;
pub(super) use transition::{cuts, refusal, selected_cut};

use super::{AssetsTab, Editor, Result};

impl Editor {
    pub(in crate::editor) fn remember_assets_tab(&mut self, tab: AssetsTab) -> Result<()> {
        if self.preferences.assets_tab != tab {
            let flushed = if let Some(panel) = &mut self.subtitles {
                panel.flush_drafts(&mut self.session)?
            } else {
                false
            };
            if flushed {
                self.desk().sync();
            }
            self.preferences
                .set("assets_tab", serde_json::to_value(tab)?)?;
        }
        Ok(())
    }
}

mod rows;
pub(super) use rows::{asset_kind, asset_row, asset_state_color, file_name};

mod effects;

mod feedback;

mod marquee;
