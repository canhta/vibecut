//! Maintains asset selection.

use super::super::state::{MEDIA_REVEAL, MediaReveal};
use super::super::{
    Arc, AssetId, AssetState, AssetsTab, BTreeMap, BTreeSet, Command, Error, Result, SourceAsset,
    Text, UnsupportedReason, egui, error_detail, media_panel,
};
use super::State;
use crate::editor::Desk;
use crate::i18n;

impl State {
    pub(in crate::editor) fn prune_media_selection(&mut self, session: &engine::Session) {
        let existing: BTreeSet<AssetId> = session
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
    pub(in crate::editor) fn refresh_media_sizes(&mut self, session: &engine::Session) {
        let assets = &session.project().data().assets;
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
    pub(in crate::editor) fn media_order(
        &self,
        session: &engine::Session,
        preferences: &crate::preferences::Preferences,
    ) -> Vec<AssetId> {
        media_panel::ordered_assets(
            &session.project().data().assets,
            preferences.media_sort,
            &self.media_size_map(),
        )
    }
    pub(in crate::editor) fn media_selection(&self) -> &BTreeSet<AssetId> {
        &self.media_selection
    }
    pub(in crate::editor) fn has_media_selection(&self) -> bool {
        !self.media_selection.is_empty()
    }
    pub(in crate::editor) fn media_highlight(&self) -> Option<AssetId> {
        self.media_reveal.map(|reveal| reveal.asset)
    }
    pub(in crate::editor) fn media_delete_pending(&self) -> Vec<AssetId> {
        self.media_delete_confirm.clone().unwrap_or_default()
    }
    /// Upload one decoded thumbnail on first use; later frames reuse the handle.
    pub(in crate::editor) fn thumbnail_texture(
        &self,
        ctx: &egui::Context,
        id: AssetId,
    ) -> Option<egui::TextureHandle> {
        crate::editor::context::Thumbnails {
            images: &self.thumbnails,
            textures: &self.thumbnail_textures,
        }
        .texture(ctx, id)
    }
    pub(in crate::editor) fn prune_asset_state(&mut self, session: &engine::Session) {
        let existing: BTreeSet<AssetId> = session
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
            queue.retain_assets(&session.project().data().assets);
        }
    }
    pub(in crate::editor) fn restore_asset_state(&mut self, desk: &mut Desk<'_>) {
        if self.importing() {
            return;
        }
        let asset = desk
            .editor()
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
            let result = self.start_import_paths(desk, vec![asset.path().to_owned()], id.0);
            if result.is_err() {
                self.restoring_asset = None;
                self.asset_states
                    .insert(id, AssetState::Unsupported(UnsupportedReason::OpenFailed));
            }
        }
    }
    /// Replace the media selection; tests and agents use the same state as clicks.
    pub(in crate::editor) fn select_media(&mut self, ids: impl IntoIterator<Item = AssetId>) {
        self.media_selection = ids.into_iter().collect();
        self.media_anchor = self.media_selection.iter().next().copied();
    }
    pub(in crate::editor) fn select_asset(
        &mut self,
        desk: &mut Desk<'_>,
        id: AssetId,
        modifiers: egui::Modifiers,
    ) {
        let ids = self.media_order(desk.view().session, desk.view().preferences);
        crate::editor::selection::click(
            &mut self.media_selection,
            &mut self.media_anchor,
            id,
            modifiers.command || modifiers.ctrl,
            modifiers.shift.then_some(ids.as_slice()),
        );
    }
    /// App action `media.reveal`: switch to Media, scroll and highlight for 1.5 s.
    /// It changes presentation state only; the project snapshot is untouched.
    pub(in crate::editor) fn reveal_media(
        &mut self,
        desk: &mut Desk<'_>,
        asset: AssetId,
    ) -> Result<()> {
        if !desk
            .view()
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
        if desk.view().preferences.assets_tab != AssetsTab::Media {
            desk.editor()
                .preferences
                .set("assets_tab", serde_json::to_value(AssetsTab::Media)?)?;
        }
        self.media_reveal = Some(MediaReveal {
            asset,
            until: desk.editor().clock.now() + MEDIA_REVEAL,
        });
        Ok(())
    }
    pub(in crate::editor) fn request_media_delete(
        &mut self,
        session: &engine::Session,
        ids: Vec<AssetId>,
    ) {
        let existing: BTreeSet<AssetId> = session
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
    pub(in crate::editor) fn cancel_media_delete(&mut self) {
        self.media_delete_confirm = None;
    }
    pub(in crate::editor) fn confirm_media_delete(&mut self, desk: &mut Desk<'_>) -> Result<()> {
        let Some(ids) = self.media_delete_confirm.take() else {
            return Ok(());
        };
        let count = ids.len() as u64;
        let before = desk.editor().session.project().clone();
        match desk
            .editor()
            .session
            .apply(Command::RemoveAssets { assets: ids })
        {
            Ok(_) => {
                self.media_selection.clear();
                self.media_anchor = None;
                self.media_reveal = None;
                self.prune_media_selection(&desk.editor().session);
                if !Arc::ptr_eq(&before, desk.editor().session.project()) {
                    desk.sync();
                }
                let text = i18n::format_count(
                    desk.editor().preferences.locale,
                    count,
                    Text::MediaDeletedOne,
                    Text::MediaDeletedOther,
                );
                desk.toast(&text, None);
                Ok(())
            }
            Err(error) => {
                let detail = error_detail(&error);
                self.import_notice = Some(detail.clone());
                let locale = desk.editor().preferences.locale;
                let text = i18n::text(locale, Text::DeleteMediaFailed);
                desk.toast(text, Some(detail));
                Ok(())
            }
        }
    }
}
