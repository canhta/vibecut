//! Routes editor actions through the facade.

use crate::editor::{
    ActionTarget, Arc, AssetId, Editor, Error, ItemId, Locale, MediaView, PREFERENCE_COMMAND,
    Preferences, Result, SelectionScopes, TICKS_PER_SECOND, Text, Tick, UiAction, ZOOM_STEP,
    actions, media_panel, preview,
};

impl Editor {
    /// All control inputs (including tests/agents) use the same action path.
    /// Project edits are engine registry calls; transport/preferences are app state.
    pub fn activate(&mut self, id: &str) -> Result<()> {
        let (spec, argument) = actions::resolve(id).ok_or(Error::Unsupported("widget action"))?;
        if let Some(ticket) = spec.pending_ticket() {
            return Err(Error::NotAvailable(ticket));
        }
        self.activate_with(Some(id), |editor| {
            editor.activate_action(id, spec, argument)
        })
    }

    /// Apply an admitted input through the editor's shared activation lifecycle.
    /// Socket inputs omit the widget id; engine admission remains inside `execute`.
    pub(crate) fn activate_with<T>(
        &mut self,
        id: Option<&str>,
        execute: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let flushed = if let Some(subtitles) = &mut self.subtitles {
            if matches!(
                id.unwrap_or_default(),
                "subtitles.dismiss"
                    | "subtitles.cancel"
                    | "subtitles.consent.approve"
                    | "subtitles.consent.deny"
            ) {
                false
            } else {
                subtitles.flush_drafts(&mut self.session)?
            }
        } else {
            false
        };
        if flushed {
            self.sync_preview();
        }
        if let Some(id) = id
            && !self.action_enabled(id)
        {
            let (spec, _) = actions::resolve(id).ok_or(Error::Unsupported("widget action"))?;
            return Err(Error::ActionDisabled(spec.id));
        }
        self.session.set_selection(self.selection.clone())?;
        self.remember_view()?;
        let before = self.session.project().clone();
        let result = execute(self)?;
        if id.is_none() {
            self.selection = self.session.selection().clone();
            self.prune_selection();
        }
        self.prune_media_selection();
        if !Arc::ptr_eq(&before, self.session.project()) {
            self.sync_preview();
        }
        self.remember_view()?;
        Ok(result)
    }

    fn activate_action(
        &mut self,
        id: &str,
        spec: &actions::ActionSpec,
        argument: Option<u64>,
    ) -> Result<()> {
        let at = self.playback.position()?;
        let duration = self.session.project().duration();
        let playing = self.playback.playing();
        match spec.target {
            ActionTarget::Engine(route) => self.execute_engine_action(route)?,
            ActionTarget::Pending { ticket, .. } => return Err(Error::NotAvailable(ticket)),
            ActionTarget::Ui { action, .. } => match action {
                UiAction::MediaView => {
                    let view = match self.preferences.media_view {
                        MediaView::Grid => MediaView::List,
                        MediaView::List => MediaView::Grid,
                    };
                    self.dispatch(
                        PREFERENCE_COMMAND,
                        serde_json::json!({"key":"media_view", "value":view}),
                    )?;
                }
                UiAction::MediaSort | UiAction::MediaSortBy(_) => {
                    let key = match action {
                        UiAction::MediaSortBy(key) => key,
                        _ => self.preferences.media_sort.key,
                    };
                    let next = media_panel::next_sort(self.preferences.media_sort, key);
                    self.dispatch(
                        PREFERENCE_COMMAND,
                        serde_json::json!({"key":"media_sort", "value":next}),
                    )?;
                }
                UiAction::RequestDeleteMedia => {
                    self.request_media_delete(self.media_selection.iter().copied().collect())
                }
                UiAction::CancelDeleteMedia => self.media_delete_confirm = None,
                UiAction::ConfirmDeleteMedia => self.confirm_media_delete()?,
                UiAction::RevealMedia => {
                    let asset = AssetId(argument.ok_or(Error::Unsupported("media identity"))?);
                    self.reveal_media(asset)?;
                }
                UiAction::InsertText => self.insert_text(),
                UiAction::Subtitles | UiAction::SubtitleCue => {
                    self.prepare_subtitles()?;
                    let mut options = self.preferences.subtitles.clone();
                    if let Some(panel) = &mut self.subtitles {
                        panel.activate(
                            &mut self.session,
                            &mut options,
                            self.selection.iter().next().copied(),
                            at,
                            id,
                        )?;
                        let seek = panel.requested_seek.take();
                        let model = panel.requested_model.take();
                        if let Some(at) = seek {
                            self.seek(at)?;
                        }
                        if let Some(id) = model {
                            self.open_model(&id)?;
                        }
                    }
                    if options != self.preferences.subtitles {
                        self.set_preference("subtitles", serde_json::to_value(options)?)?;
                    }
                }

                UiAction::OpenScenes => self.scenes_open = true,
                UiAction::CloseScenes => {
                    self.scenes_open = false;
                    self.scene_confirm = false;
                    self.scene_rename = None;
                }
                UiAction::SelectScenes => {
                    self.scene_select = !self.scene_select;
                    self.scene_selected.clear();
                }
                UiAction::RequestDeleteScenes => {
                    self.scene_confirm = !self.scene_selected.is_empty()
                }
                UiAction::CancelDeleteScenes => self.scene_confirm = false,
                UiAction::RenameScene => {
                    let scene =
                        project::SceneId(argument.ok_or(Error::Unsupported("scene identity"))?);
                    let name = self
                        .session
                        .project()
                        .data()
                        .scenes
                        .get(scene)
                        .ok_or(Error::Unsupported("scene not found"))?
                        .name
                        .as_str()
                        .to_owned();
                    self.scene_rename = Some((scene, name));
                    self.scene_rename_focus = true;
                }
                UiAction::SceneRow => {
                    let scene =
                        project::SceneId(argument.ok_or(Error::Unsupported("scene identity"))?);
                    let target = self
                        .session
                        .project()
                        .data()
                        .scenes
                        .get(scene)
                        .ok_or(Error::Unsupported("scene not found"))?;
                    if self.scene_select {
                        if !target.is_main && !self.scene_selected.remove(&scene) {
                            self.scene_selected.insert(scene);
                        }
                    } else {
                        self.cancel_interaction();
                        self.session
                            .execute("scene.switch", serde_json::json!({"scene":scene}))?;
                        self.selection.clear();
                        self.selection_scopes = SelectionScopes::default();
                    }
                }
                UiAction::Export => {
                    if self.export.is_none() {
                        self.export = Some(crate::export::ExportDialog::new(
                            self.playback.budget().clone(),
                            self.playback.render_state()?,
                        )?);
                    }
                    if let Some(dialog) = &mut self.export {
                        dialog.show(self.session.project());
                    }
                }
                UiAction::RunExport => {
                    if self.export.is_none() {
                        self.export = Some(crate::export::ExportDialog::new(
                            self.playback.budget().clone(),
                            self.playback.render_state()?,
                        )?);
                    }
                    let dialog = self
                        .export
                        .as_mut()
                        .ok_or(Error::Unsupported("export dialog"))?;
                    dialog.show(self.session.project());
                    dialog.start(self.session.project().clone(), self.preferences.export)?;
                }
                UiAction::RetryExport => {
                    let dialog = self
                        .export
                        .as_mut()
                        .ok_or(Error::Unsupported("export dialog"))?;
                    dialog.retry(self.session.project().clone())?;
                }
                UiAction::Save => {
                    self.lifecycle_action = Some(crate::lifecycle::EditorAction::Save)
                }
                UiAction::SaveAs => {
                    self.lifecycle_action = Some(crate::lifecycle::EditorAction::SaveAs)
                }
                UiAction::FocusHome => {
                    self.lifecycle_action = Some(crate::lifecycle::EditorAction::FocusHome)
                }
                UiAction::RenameProject => {
                    self.frame.rename =
                        Some(self.session.project().data().name.as_str().to_owned());
                    self.frame.rename_focus = true;
                }
                UiAction::Import => {
                    if self.importing() {
                        self.import_notice = Some(self.tr(Text::ImportBusy).to_owned());
                    } else {
                        self.lifecycle_action = Some(crate::lifecycle::EditorAction::ImportMedia);
                    }
                }
                UiAction::CancelImport => {
                    if let Some(queue) = &self.imports {
                        queue.cancel();
                    }
                }
                UiAction::Play => self.playback.seek(
                    if at >= duration { Tick::ZERO } else { at },
                    !playing,
                    self.session.project().clone(),
                )?,
                UiAction::Pause => self
                    .playback
                    .seek(at, false, self.session.project().clone())?,
                UiAction::BackSecond => {
                    self.seek((at - Tick(TICKS_PER_SECOND)).max(Tick::ZERO))?;
                }
                UiAction::ForwardSecond => {
                    self.seek((at + Tick(TICKS_PER_SECOND)).min(duration))?;
                }
                UiAction::FrameBack => self.step_frames(-1)?,
                UiAction::FrameForward => self.step_frames(1)?,
                UiAction::Start => self.seek(Tick::ZERO)?,
                UiAction::End => self.seek(duration)?,
                UiAction::Undo | UiAction::Redo => {
                    let preference_changed = if action == UiAction::Undo {
                        self.preference_history.undo(&mut self.preferences)
                    } else {
                        self.preference_history.redo(&mut self.preferences)
                    };
                    if !preference_changed {
                        self.session.execute(
                            if action == UiAction::Undo {
                                engine::CommandKind::Undo
                            } else {
                                engine::CommandKind::Redo
                            }
                            .id(),
                            serde_json::json!({}),
                        )?;
                        self.selection = self.session.selection().clone();
                        self.selection_scopes = SelectionScopes::default();
                        self.property_drafts.clear();
                    } else {
                        self.sync_keymap();
                    }
                }
                UiAction::SelectAll => self.select_all(),
                UiAction::DeselectAll => {
                    self.selection.clear();
                    self.selection_scopes = SelectionScopes::default();
                }
                UiAction::Escape => self.cancel_interaction(),
                UiAction::Snap => {
                    let after = !self.preferences.snapping;
                    self.dispatch(
                        PREFERENCE_COMMAND,
                        serde_json::json!({"key":"snapping", "value":after}),
                    )?;
                }
                UiAction::Ripple => self.ripple = !self.ripple,
                UiAction::ZoomIn => self.zoom_by(ZOOM_STEP),
                UiAction::ZoomOut => self.zoom_by(1.0 / ZOOM_STEP),
                UiAction::Theme => {
                    let after = !self.preferences.dark;
                    self.dispatch(
                        PREFERENCE_COMMAND,
                        serde_json::json!({"key": "dark", "value": after}),
                    )?;
                }
                UiAction::Locale => {
                    let after = if self.preferences.locale == Locale::En {
                        Locale::Vi
                    } else {
                        Locale::En
                    };
                    self.dispatch(
                        PREFERENCE_COMMAND,
                        serde_json::json!({"key": "locale", "value": after}),
                    )?;
                }
                UiAction::ResetLayout => {
                    self.dispatch(
                        PREFERENCE_COMMAND,
                        serde_json::json!({
                            "key": "panels",
                            "value": Preferences::default().panels,
                        }),
                    )?;
                }
                UiAction::OpenModels => self.open_models()?,
                UiAction::OpenShortcuts => self.open_shortcuts(),
                UiAction::RetryPreview => {
                    self.playback.clear()?;
                    self.error = None;
                }
                UiAction::ProjectInspector => self.details = false,
                UiAction::DetailsInspector => self.details = true,
                UiAction::PreviewSnapshot => self.open_snapshot_dialog()?,
                UiAction::PreviewFullscreen => self.preview_fullscreen = !self.preview_fullscreen,
                UiAction::PreviewFit => self.set_preview_fit()?,
                UiAction::PreviewZoomIn => self.preview_zoom_by(preview::ZOOM_STEP)?,
                UiAction::PreviewZoomOut => self.preview_zoom_by(1.0 / preview::ZOOM_STEP)?,
                UiAction::PreviewZoomPreset => {
                    let percent =
                        u32::try_from(argument.ok_or(Error::Unsupported("preview zoom"))?)
                            .map_err(|_| Error::Unsupported("preview zoom"))?;
                    let viewport = self.preview_viewport;
                    let canvas = self.preview_canvas_size();
                    let zoom = preview::normalize_zoom(percent as f32 / 100.0)
                        .ok_or(Error::Invalid("preview zoom"))?;
                    self.set_preview_zoom(zoom, viewport, canvas)?;
                }
                UiAction::SelectItem => {
                    let item = ItemId(argument.ok_or(Error::Unsupported("item identity"))?);
                    if !self
                        .session
                        .project()
                        .data()
                        .scene()
                        .timeline
                        .iter()
                        .any(|t| t.items.iter().any(|i| i.id == item))
                    {
                        return Err(Error::Engine(engine::EditError::NoItem(item)));
                    }
                    self.select_single(item);
                }
            },
        }
        Ok(())
    }
}
