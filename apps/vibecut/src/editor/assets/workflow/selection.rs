//! Maintains asset selection.

use crate::editor::assets::{
    Arc, AssetId, AssetState, AssetsTab, BTreeMap, BTreeSet, Command, Editor, Error, MEDIA_REVEAL,
    MediaReveal, Result, SourceAsset, Text, UnsupportedReason, egui, error_detail, media_panel,
};
use crate::i18n;

impl Editor {
    pub(in crate::editor) fn prune_media_selection(&mut self) {
        let existing: BTreeSet<AssetId> = self
            .session
            .project()
            .data()
            .assets
            .iter()
            .filter(|asset| !asset.ephemeral())
            .map(SourceAsset::id)
            .collect();
        self.media_selection.retain(|id| existing.contains(id));
        if self
            .media_anchor
            .is_some_and(|anchor| !existing.contains(&anchor))
        {
            self.media_anchor = None;
        }
        self.media_sizes.retain(|id, _| existing.contains(id));
        if self
            .media_reveal
            .is_some_and(|reveal| !existing.contains(&reveal.asset))
        {
            self.media_reveal = None;
        }
        if let Some(pending) = &mut self.media_delete_confirm {
            pending.retain(|id| existing.contains(id));
            if pending.is_empty() {
                self.media_delete_confirm = None;
            }
        }
    }
    pub(in crate::editor) fn media_size_map(&self) -> BTreeMap<AssetId, Option<u64>> {
        self.media_sizes
            .iter()
            .map(|(id, (_, size))| (*id, *size))
            .collect()
    }
    /// File size is display metadata; stat each asset once per content identity.
    pub(in crate::editor) fn refresh_media_sizes(&mut self) {
        let assets = &self.session.project().data().assets;
        let mut next = BTreeMap::new();
        for asset in assets.iter().filter(|asset| !asset.ephemeral()) {
            let hash = asset.content_hash();
            let size = match self.media_sizes.get(&asset.id()) {
                Some((cached, Some(size))) if *cached == hash => Some(*size),
                Some((cached, None)) if *cached == hash => media_panel::file_size(asset.path()),
                _ => media_panel::file_size(asset.path()),
            };
            next.insert(asset.id(), (hash, size));
        }
        self.media_sizes = next;
    }
    pub(in crate::editor) fn media_order(&self) -> Vec<AssetId> {
        media_panel::ordered_assets(
            &self.session.project().data().assets,
            self.preferences.media_sort,
            &self.media_size_map(),
        )
    }
    pub(in crate::editor) fn prune_asset_state(&mut self) {
        let existing: BTreeSet<AssetId> = self
            .session
            .project()
            .data()
            .assets
            .iter()
            .map(SourceAsset::id)
            .collect();
        self.asset_states.retain(|id, _| existing.contains(id));
        self.thumbnails.retain(|id, _| existing.contains(id));
        self.thumbnail_textures
            .get_mut()
            .retain(|id, _| existing.contains(id));
        if let Some(queue) = &mut self.imports {
            queue.retain_assets(&self.session.project().data().assets);
        }
    }
    pub(in crate::editor) fn restore_asset_state(&mut self) {
        if self.importing() {
            return;
        }
        let asset = self
            .session
            .project()
            .data()
            .assets
            .iter()
            .find(|a| !a.ephemeral() && !self.asset_states.contains_key(&a.id()))
            .cloned();
        if let Some(asset) = asset {
            let id = asset.id();
            self.asset_states.insert(id, AssetState::Probing);
            self.restoring_asset = Some(id);
            let result = self.start_import_paths(vec![asset.path().to_owned()], id.0);
            if result.is_err() {
                self.restoring_asset = None;
                self.asset_states
                    .insert(id, AssetState::Unsupported(UnsupportedReason::OpenFailed));
            }
        }
    }
    /// Replace the media selection; tests and agents use the same state as clicks.
    pub fn select_media(&mut self, ids: impl IntoIterator<Item = AssetId>) {
        self.media_selection = ids.into_iter().collect();
        self.media_anchor = self.media_selection.iter().next().copied();
        self.session.end_merge();
    }
    pub(in crate::editor) fn select_asset(&mut self, id: AssetId, modifiers: egui::Modifiers) {
        let ids = self.media_order();
        crate::selection::click(
            &mut self.media_selection,
            &mut self.media_anchor,
            id,
            modifiers.command || modifiers.ctrl,
            modifiers.shift.then_some(ids.as_slice()),
        );
        self.session.end_merge();
    }
    /// App action `media.reveal`: switch to Media, scroll and highlight for 1.5 s.
    /// It changes presentation state only; the project snapshot is untouched.
    pub fn reveal_media(&mut self, asset: AssetId) -> Result<()> {
        if !self
            .session
            .project()
            .data()
            .assets
            .iter()
            .any(|candidate| candidate.id() == asset && !candidate.ephemeral())
        {
            return Err(Error::Engine(engine::control::Error::Edit(
                engine::EditError::NoAsset(asset),
            )));
        }
        if self.preferences.assets_tab != AssetsTab::Media {
            self.preferences
                .set("assets_tab", serde_json::to_value(AssetsTab::Media)?)?;
        }
        self.media_reveal = Some(MediaReveal {
            asset,
            until: self.clock.now() + MEDIA_REVEAL,
        });
        self.session.end_merge();
        Ok(())
    }
    pub(in crate::editor) fn request_media_delete(&mut self, ids: Vec<AssetId>) {
        let existing: BTreeSet<AssetId> = self
            .session
            .project()
            .data()
            .assets
            .iter()
            .filter(|asset| !asset.ephemeral())
            .map(SourceAsset::id)
            .collect();
        let mut ids: Vec<AssetId> = ids.into_iter().filter(|id| existing.contains(id)).collect();
        ids.sort_unstable();
        ids.dedup();
        if !ids.is_empty() {
            self.media_delete_confirm = Some(ids);
        }
    }
    pub(in crate::editor) fn confirm_media_delete(&mut self) -> Result<()> {
        let Some(ids) = self.media_delete_confirm.take() else {
            return Ok(());
        };
        let count = ids.len() as u64;
        let before = self.session.project().clone();
        match self.session.apply(Command::RemoveAssets { assets: ids }) {
            Ok(_) => {
                self.media_selection.clear();
                self.media_anchor = None;
                self.media_reveal = None;
                self.prune_media_selection();
                if !Arc::ptr_eq(&before, self.session.project()) {
                    self.desk().sync();
                }
                let text = i18n::format_count(
                    self.preferences.locale,
                    count,
                    Text::MediaDeletedOne,
                    Text::MediaDeletedOther,
                );
                self.set_toast(&text, None);
                Ok(())
            }
            Err(error) => {
                let detail = error_detail(&error);
                self.import_notice = Some(detail.clone());
                self.set_toast(self.tr(Text::DeleteMediaFailed), Some(detail));
                Ok(())
            }
        }
    }
}
