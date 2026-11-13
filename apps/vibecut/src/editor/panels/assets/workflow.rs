//! Applies the assets workflows.
mod actions;
mod adjustment;
mod effects;
mod import;
mod insertion;
mod marquee;
mod music;
mod rows;
mod selection;
mod sounds;
mod stickers;
mod transition;
pub(super) use transition::{cuts, refusal, selected_cut};

use super::{AssetsTab, State};
use crate::editor::{Port, Result};

impl State {
    pub(in crate::editor) fn remember_assets_tab(
        &mut self,
        port: &mut Port<'_>,
        tab: AssetsTab,
    ) -> Result<()> {
        if port.read.preferences.assets_tab == tab {
            return Ok(());
        }
        let flushed = if let Some(panel) = &mut self.captions {
            panel.flush_drafts(&mut *port.session)?
        } else {
            false
        };
        if flushed {
            port.sync();
        }
        port.store_preference("assets_tab", serde_json::to_value(tab)?);
        Ok(())
    }
}

pub(super) use rows::{asset_kind, asset_row, asset_state_color, file_name};
