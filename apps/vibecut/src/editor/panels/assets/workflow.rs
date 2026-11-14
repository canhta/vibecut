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

use super::{AssetsTab, CaptionsSection, State};
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

    /// Open the captions transcript pane on one clip: show the Captions tab's Auto page and
    /// select the words whose output range overlaps the clip.
    pub(in crate::editor) fn open_transcript(
        &mut self,
        port: &mut Port<'_>,
        item: project::ItemId,
    ) -> Result<()> {
        let span = port
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)
            .and_then(|candidate| candidate.end().ok().map(|end| (candidate.start, end)))
            .ok_or(crate::editor::Error::Invalid("transcript target"))?;
        self.remember_assets_tab(port, AssetsTab::Captions)?;
        self.captions_section = CaptionsSection::Auto;
        self.prepare_captions(port.read.budget)?;
        if let Some(panel) = &mut self.captions {
            panel.select_words_in(&*port.session, span);
        }
        Ok(())
    }
}

pub(super) use rows::{asset_kind, asset_row, asset_state_color, file_name};
