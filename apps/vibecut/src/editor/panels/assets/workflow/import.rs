//! Applies media import results.

use super::super::import::needs_thumbnail;
use super::super::{
    Arc, AssetId, AssetState, Command, Error, ImportEvent, ImportFailure, ImportProbe, ImportQueue,
    PathBuf, Result, SourceAsset, Text, ThumbnailOutcome, ThumbnailSource, UnsupportedReason,
    error_detail,
};
use super::State;
use crate::editor::Port;
use crate::i18n;

impl State {
    /// Replace the media boundary in tests; production keeps the media owner.
    pub(in crate::editor) fn set_import_probe(
        &mut self,
        port: &mut Port<'_>,
        probe: Arc<dyn ImportProbe>,
    ) -> Result<()> {
        self.import_queue(port)?.set_probe(probe);
        Ok(())
    }
    /// Replace thumbnail decoding in tests; production reuses media's decoder.
    pub(in crate::editor) fn set_thumbnailer(
        &mut self,
        port: &mut Port<'_>,
        thumbnailer: Arc<dyn ThumbnailSource>,
    ) -> Result<()> {
        self.import_queue(port)?.set_thumbnailer(thumbnailer);
        Ok(())
    }
    /// Persistent content-keyed thumbnail directory. Without one thumbnails are
    /// memoized for this session only.
    pub(in crate::editor) fn set_thumbnail_root(&mut self, root: PathBuf) {
        if let Some(queue) = &mut self.imports {
            queue.set_thumbnail_root(root.clone());
        }
        self.thumbnail_root = Some(root);
    }
    pub(in crate::editor) fn importing(&self) -> bool {
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
    pub(in crate::editor) fn start_import(
        &mut self,
        port: &mut Port<'_>,
        paths: Vec<PathBuf>,
    ) -> Result<()> {
        if self.importing() {
            return Err(Error::Invalid("import already in progress"));
        }
        if paths.is_empty() {
            return Ok(());
        }
        let first = self.next_asset_id(port.session);
        self.import_total = paths.len();
        self.import_completed = 0;
        self.import_imported = 0;
        self.import_cancelled = false;
        self.import_failures.clear();
        self.import_notice = None;
        self.import_error = None;
        self.import_added.clear();
        self.paste_at = None;
        port.clear_toast();
        let result = self.start_import_paths(port, paths, first);
        if result.is_err() {
            self.import_total = 0;
        }
        result
    }
    pub(in crate::editor) fn start_import_paths(
        &mut self,
        port: &mut Port<'_>,
        paths: Vec<PathBuf>,
        first: u64,
    ) -> Result<()> {
        let result = self.import_queue(port)?.start(paths.clone(), first);
        if matches!(
            result,
            Err(Error::Engine(engine::control::Error::Runtime(
                engine::RuntimeError::BudgetFull
            )))
        ) {
            self.thumbnail_textures.get_mut().clear();
            self.thumbnails.clear();
            self.import_queue(port)?.evict_thumbnails();
            self.import_queue(port)?.start(paths, first)
        } else {
            result
        }
    }
    pub(in crate::editor) fn import_queue(
        &mut self,
        port: &mut Port<'_>,
    ) -> Result<&mut ImportQueue> {
        if self.imports.is_none() {
            let budget = port.read.budget.clone();
            self.imports = Some(ImportQueue::new(budget)?);
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
    fn next_asset_id(&self, session: &engine::Session) -> u64 {
        session
            .project()
            .data()
            .assets
            .iter()
            .map(|asset| asset.id().0)
            .max()
            .map_or(1, |id| id.saturating_add(1))
    }
    fn add_imported_asset(&mut self, port: &mut Port<'_>, asset: SourceAsset) -> Result<AssetId> {
        // The worker reserves this identity before publishing its probing row;
        // undoing an earlier import must not renumber later job/thumbnail events.
        let id = asset.id();
        port.session.apply(Command::AddAsset { asset })?;
        Ok(id)
    }
    pub(in crate::editor) fn apply_import_events(&mut self, port: &mut Port<'_>) {
        let before = port.session.project().clone();
        let polled = match self.imports.as_mut() {
            Some(queue) => queue.poll(),
            None => Ok(Vec::new()),
        };
        let events = match polled {
            Ok(events) => events,
            Err(_) if self.restoring_asset.is_some() => {
                self.finish_import(port);
                return;
            }
            Err(error) => {
                self.import_notice = Some(error_detail(error.as_ref()));
                self.import_error = Some(error);
                if let Some(queue) = &self.imports {
                    queue.cancel();
                }
                self.finish_import(port);
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
                    let thumbnail = needs_thumbnail(&imported);
                    let adding = if self.restoring_asset == Some(imported.asset.id()) {
                        let id = imported.asset.id();
                        let matches = port.session.project().data().assets.iter().any(|a| {
                            a.id() == id && a.content_hash() == imported.asset.content_hash()
                        });
                        if matches {
                            Ok(id)
                        } else {
                            Err(Error::Invalid("restored media content changed"))
                        }
                    } else {
                        self.add_imported_asset(port, imported.asset)
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
                                self.import_added.push(id);
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
                ImportEvent::Done { .. } if self.restoring_asset.is_some() => {
                    self.finish_import(port);
                }
                ImportEvent::Done { cancelled, error } => {
                    self.import_cancelled = cancelled;
                    if let Some(error) = error {
                        self.import_notice = Some(error_detail(error.as_ref()));
                        self.import_error = Some(error);
                    }
                    self.finish_import(port);
                }
            }
        }
        self.prune_asset_state(port.session);
        self.prune_media_selection(port.session);
        if !Arc::ptr_eq(&before, port.session.project()) {
            port.sync();
        }
    }
    pub(in crate::editor) fn finish_import(&mut self, port: &mut Port<'_>) {
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
        if let Some(at) = self.paste_at.take() {
            self.insert_pasted(port, at);
        }
        let failed_count = self.import_failures.len();
        let unfinished_count = self.import_total.saturating_sub(self.import_completed);
        let locale = port.locale();
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
            i18n::text(locale, Text::ImportFailed)
        } else if self.import_cancelled {
            i18n::text(locale, Text::ImportCancelled)
        } else if failed_count > 0 || unfinished_count > 0 {
            i18n::text(locale, Text::ImportPartial)
        } else {
            i18n::text(locale, Text::ImportComplete)
        };
        port.toast(text, Some(detail));
        self.import_total = self.import_total.max(self.import_completed);
    }
}
