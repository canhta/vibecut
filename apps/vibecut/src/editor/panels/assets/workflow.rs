//! Applies the assets workflows.
mod effects;
mod import;
mod insertion;
mod marquee;
mod music;
mod rows;
mod selection;
mod sounds;
mod transition;
pub(super) use transition::{cuts, refusal, selected_cut};

use super::{AssetsTab, State};
use crate::editor::{Desk, Result};

impl State {
    pub(in crate::editor) fn remember_assets_tab(
        &self,
        desk: &mut Desk<'_>,
        tab: AssetsTab,
    ) -> Result<()> {
        let flushed = {
            let editor = desk.editor();
            if editor.preferences.assets_tab == tab {
                return Ok(());
            }
            if let Some(panel) = &mut editor.subtitles {
                panel.flush_drafts(&mut editor.session)?
            } else {
                false
            }
        };
        if flushed {
            desk.sync();
        }
        desk.editor()
            .preferences
            .set("assets_tab", serde_json::to_value(tab)?)?;
        Ok(())
    }
}

pub(super) use rows::{asset_kind, asset_row, asset_state_color, file_name};
