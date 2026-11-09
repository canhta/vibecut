//! Applies media import results.

use crate::editor::assets::{
    Arc, AssetId, AssetState, Command, Editor, Error, ImportEvent, ImportFailure, ImportProbe,
    ImportQueue, PathBuf, Result, SourceAsset, Text, ThumbnailOutcome, ThumbnailSource,
    UnsupportedReason, error_detail,
};
use crate::i18n;

impl Editor {
    /// Replace the media boundary in tests; production keeps the media owner.
    pub fn set_import_probe(&mut self, probe: Arc<dyn ImportProbe>) -> Result<()> {
        self.import_queue()?.set_probe(probe);
        Ok(())
    }
    /// Replace thumbnail decoding in tests; production reuses media's decoder.
    pub fn set_thumbnailer(&mut self, thumbnailer: Arc<dyn ThumbnailSource>) -> Result<()> {
        self.import_queue()?.set_thumbnailer(thumbnailer);
        Ok(())
    }
    /// Persistent content-keyed thumbnail directory. Without one thumbnails are
    /// memoized for this session only.
    pub fn set_thumbnail_root(&mut self, root: PathBuf) {
        if let Some(queue) = &mut self.imports {
            queue.set_thumbnail_root(root.clone());
        }
        self.thumbnail_root = Some(root);
    }
    pub fn importing(&self) -> bool {
        self.imports.as_ref().is_some_and(ImportQueue::active)
    }
    /// Batch fraction for the import overlay: the probe job's current file
    /// fraction while probing, otherwise completed files over the batch.
    pub(in crate::editor) fn import_fraction(&self) -> f32 {
        self.imports
            .as_ref()
            .and_then(ImportQueue::progress)
            .unwrap_or_else(|| self.import_completed as f32 / self.import_total.max(1) as f32)
            .clamp(0.0, 1.0)
    }
    /// Accept a native drag-drop batch through the same path as the picker.
    pub fn drop_files(&mut self, paths: Vec<PathBuf>) -> Result<()> {
        self.start_import(paths)
    }
    pub fn start_import(&mut self, paths: Vec<PathBuf>) -> Result<()> {
        if self.importing() {
            return Err(Error::Invalid("import already in progress"));
        }
        if paths.is_empty() {
            return Ok(());
        }
        let first = self.next_asset_id();
        self.import_total = paths.len();
        self.import_completed = 0;
        self.import_imported = 0;
        self.import_cancelled = false;
        self.import_failures.clear();
        self.import_notice = None;
        self.import_error = None;
        self.toast = None;
        let result = self.start_import_paths(paths, first);
        if result.is_err() {
            self.import_total = 0;
        }
        result
    }
    pub(in crate::editor) fn start_import_paths(
        &mut self,
        paths: Vec<PathBuf>,
        first: u64,
    ) -> Result<()> {
        let result = self.import_queue()?.start(paths.clone(), first);
        if matches!(result, Err(Error::Engine(engine::EditError::BudgetFull))) {
            self.thumbnail_textures.get_mut().clear();
            self.thumbnails.clear();
            self.import_queue()?.evict_thumbnails();
            self.import_queue()?.start(paths, first)
        } else {
            result
        }
    }
    pub(in crate::editor) fn import_queue(&mut self) -> Result<&mut ImportQueue> {
        if self.imports.is_none() {
            self.imports = Some(ImportQueue::new(self.playback.budget().clone())?);
            if let Some(root) = &self.thumbnail_root
                && let Some(queue) = &mut self.imports
            {
                queue.set_thumbnail_root(root.clone());
            }
        }
        self.imports
            .as_mut()
            .ok_or(Error::Invalid("import queue unavailable"))
    }
    pub(in crate::editor) fn next_asset_id(&self) -> u64 {
        self.session
            .project()
            .data()
            .assets
            .iter()
            .map(|asset| asset.id().0)
            .max()
            .map_or(1, |id| id.saturating_add(1))
    }
    pub(in crate::editor) fn add_imported_asset(&mut self, asset: SourceAsset) -> Result<AssetId> {
        // The worker reserves this identity before publishing its probing row;
        // undoing an earlier import must not renumber later job/thumbnail events.
        let id = asset.id();
        self.session.apply(Command::AddAsset { asset })?;
        Ok(id)
    }
    pub(in crate::editor) fn apply_import_events(&mut self) {
        let before = self.session.project().clone();
        let polled = match self.imports.as_mut() {
            Some(queue) => queue.poll(),
            None => Ok(Vec::new()),
        };
        let events = match polled {
            Ok(events) => events,
            Err(_) if self.restoring_asset.is_some() => {
                self.finish_import();
                return;
            }
            Err(error) => {
                self.import_notice = Some(error_detail(error.as_ref()));
                self.import_error = Some(error);
                if let Some(queue) = &self.imports {
                    queue.cancel();
                }
                self.finish_import();
                return;
            }
        };
        for event in events {
            match event {
                ImportEvent::Probing { id, path } => {
                    if self.restoring_asset != Some(id) {
                        self.pending_imports.insert(id, path);
                    }
                }
                ImportEvent::Imported(imported) => {
                    self.pending_imports.remove(&imported.asset.id());
                    self.pending_imports
                        .retain(|_, path| path != imported.asset.path());
                    let path = imported.asset.path().to_owned();
                    if self.restoring_asset.is_none() {
                        self.import_completed += 1;
                    }
                    let unsupported = imported.capabilities.unsupported();
                    let thumbnail = crate::import::needs_thumbnail(&imported);
                    let adding = if self.restoring_asset == Some(imported.asset.id()) {
                        let id = imported.asset.id();
                        let matches = self.session.project().data().assets.iter().any(|a| {
                            a.id() == id && a.content_hash() == imported.asset.content_hash()
                        });
                        if matches {
                            Ok(id)
                        } else {
                            Err(Error::Invalid("restored media content changed"))
                        }
                    } else {
                        self.add_imported_asset(imported.asset)
                    };
                    match adding {
                        Ok(id) => {
                            let state = match unsupported {
                                Some(reason) => AssetState::Unsupported(reason),
                                None if thumbnail && self.restoring_asset.is_none() => {
                                    AssetState::Probing
                                }
                                None => AssetState::Ready,
                            };
                            self.asset_states.insert(id, state);
                            if self.restoring_asset.is_none() {
                                self.import_imported += 1;
                            }
                        }
                        Err(error) => {
                            if let Some(id) = self.restoring_asset {
                                self.asset_states.insert(
                                    id,
                                    AssetState::Unsupported(UnsupportedReason::OpenFailed),
                                );
                            }
                            if self.restoring_asset.is_none() {
                                self.import_failures
                                    .push(ImportFailure::local(path, error_detail(&error)));
                            }
                        }
                    }
                }
                ImportEvent::Failed(failure) => {
                    self.pending_imports.retain(|_, path| path != &failure.path);
                    if let Some(id) = self.restoring_asset {
                        self.asset_states
                            .insert(id, AssetState::Unsupported(UnsupportedReason::OpenFailed));
                    }
                    if self.restoring_asset.is_none() {
                        self.import_completed += 1;
                        self.import_failures.push(failure);
                    }
                }
                ImportEvent::Thumbnail { asset, .. }
                    if !matches!(
                        self.asset_states.get(&asset),
                        Some(AssetState::Probing | AssetState::Ready)
                    ) => {}
                ImportEvent::Thumbnail { asset, outcome } => match outcome {
                    ThumbnailOutcome::Ready(thumbnail) => {
                        while self.thumbnails.len() >= 64 {
                            if let Some((id, _)) = self.thumbnails.pop_first() {
                                self.thumbnail_textures.get_mut().remove(&id);
                            }
                        }
                        self.thumbnails.insert(asset, thumbnail);
                        self.thumbnail_textures.get_mut().remove(&asset);
                        self.asset_states.insert(asset, AssetState::Ready);
                    }
                    ThumbnailOutcome::Failed(error) => {
                        // The asset stays usable with a placeholder.
                        self.asset_states.insert(asset, AssetState::Ready);
                        tracing::warn!(category = "thumbnail_generation", error = %error, "thumbnail unavailable");
                    }
                },
                ImportEvent::Done { .. } if self.restoring_asset.is_some() => self.finish_import(),
                ImportEvent::Done { cancelled, error } => {
                    self.import_cancelled = cancelled;
                    if let Some(error) = error {
                        self.import_notice = Some(error_detail(error.as_ref()));
                        self.import_error = Some(error);
                    }
                    self.finish_import();
                }
            }
        }
        self.prune_asset_state();
        self.prune_media_selection();
        if !Arc::ptr_eq(&before, self.session.project()) {
            self.sync_preview();
        }
    }
    pub(in crate::editor) fn finish_import(&mut self) {
        self.pending_imports.clear();
        if let Some(id) = self.restoring_asset.take() {
            if self.asset_states.get(&id) == Some(&AssetState::Probing) {
                self.asset_states
                    .insert(id, AssetState::Unsupported(UnsupportedReason::OpenFailed));
            }
            return;
        }
        // Completed probes already validated these assets; cancellation or a
        // terminal thumbnail failure leaves a usable asset and a placeholder.
        for state in self.asset_states.values_mut() {
            if *state == AssetState::Probing {
                *state = AssetState::Ready;
            }
        }
        let failed_count = self.import_failures.len();
        let unfinished_count = self.import_total.saturating_sub(self.import_completed);
        let locale = self.preferences.locale;
        let imported = i18n::number(locale, self.import_imported as u64);
        let failed = i18n::number(locale, failed_count as u64);
        let unfinished = i18n::number(locale, unfinished_count as u64);
        let detail = i18n::format(
            locale,
            Text::ImportSummary,
            &[
                ("imported", &imported),
                ("failed", &failed),
                ("unfinished", &unfinished),
            ],
        );
        let text = if self.import_error.is_some() {
            self.tr(Text::ImportFailed)
        } else if self.import_cancelled {
            self.tr(Text::ImportCancelled)
        } else if failed_count > 0 || unfinished_count > 0 {
            self.tr(Text::ImportPartial)
        } else {
            self.tr(Text::ImportComplete)
        };
        self.set_toast(text, Some(detail));
        self.import_total = self.import_total.max(self.import_completed);
    }
}
