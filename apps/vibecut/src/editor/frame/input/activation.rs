//! Routes editor actions through the facade.

use crate::editor::{
    ActionTarget, Arc, AssetId, Editor, Error, ItemId, Locale, MediaView, PREFERENCE_COMMAND,
    Preferences, Result, SelectionScopes, Text, UiAction, ZOOM_STEP, actions, media_panel,
};
use crate::services::playback::Transport;
use crate::services::preview;

impl Editor {
    /// All control inputs (including tests/agents) use the same action path.
    /// Project edits are engine registry calls; transport/preferences are app state.
    pub fn activate(&mut self, id: &str) -> Result<()> {
        // Starting playback closes an inline edit before the engine admits the
        // action: selection and save commands refuse while a preview is open.
        if id == "transport.play" && !self.playback.playing() {
            self.commit_text_edit();
        }
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
            self.desk().sync();
        }
        if let Some(id) = id
            && !self.action_enabled(id)
        {
            let (spec, _) = actions::resolve(id).ok_or(Error::Unsupported("widget action"))?;
            return Err(Error::ActionDisabled(spec.id));
        }
        self.desk().push_selection()?;
        self.remember_view()?;
        let before = self.session.project().clone();
        let result = execute(self)?;
        if id.is_none() {
            self.selection = self.session.selection().clone();
            self.prune_selection();
        }
        self.prune_media_selection();
        if !Arc::ptr_eq(&before, self.session.project()) {
            self.desk().sync();
        }
        self.remember_view()?;
        self.drain_requests();
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
        if id == "timeline.delete"
            && self.selection_scopes.active(&self.selection)
                == crate::selection::SelectionScope::Keyframes
        {
            let keyframes = self.selection_scopes.keyframes.clone();
            self.delete_keyframes(&keyframes);
            return Ok(());
        }
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
                        crate::editor::captions::queue_requests(self);
                    }
                    if options != self.preferences.subtitles {
                        self.set_preference("subtitles", serde_json::to_value(options)?)?;
                    }
                }

                UiAction::OpenScenes => self.timeline.scenes.open(),
                UiAction::CloseScenes => self.timeline.scenes.close(),
                UiAction::SelectScenes => self.timeline.scenes.toggle_selecting(),
                UiAction::RequestDeleteScenes => self.timeline.scenes.request_delete(),
                UiAction::CancelDeleteScenes => self.timeline.scenes.cancel_delete(),
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
                    self.timeline.scenes.begin_rename(scene, name);
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
                    if self.timeline.scenes.is_selecting() {
                        let is_main = target.is_main;
                        self.timeline.scenes.toggle(scene, is_main);
                    } else {
                        self.cancel_interaction();
                        self.session.apply(engine::Command::SwitchScene { scene })?;
                        self.selection.clear();
                        self.selection_scopes = SelectionScopes::default();
                    }
                }
                UiAction::Export => self.open_export()?,
                UiAction::Crop => crate::editor::panels::player::crop::open(self)?,
                UiAction::RunExport => self.run_export(self.preferences.export)?,
                action @ (UiAction::ExportMarkIn | UiAction::ExportMarkOut) => {
                    let mut options = self.preferences.export;
                    let (mut start, mut end) = match options.output.range {
                        render::RangeChoice::InOut { start, end } => (start, end),
                        _ => (time::Tick::ZERO, duration),
                    };
                    let at = self
                        .session
                        .project()
                        .data()
                        .canvas
                        .fps
                        .snap_nearest(at.clamp(time::Tick::ZERO, duration))?;
                    if action == UiAction::ExportMarkIn {
                        start = at;
                    } else {
                        end = at;
                    }
                    options.output.scene = render::SceneChoice::Active;
                    options.output.range = render::RangeChoice::InOut { start, end };
                    self.dispatch(
                        crate::editor::PREFERENCE_COMMAND,
                        serde_json::json!({"key":"export","value":options}),
                    )?;
                }
                UiAction::ExportInOut => {
                    let mut options = self.preferences.export;
                    options.output.scene = render::SceneChoice::Active;
                    if !matches!(options.output.range, render::RangeChoice::InOut { .. }) {
                        return Err(Error::Invalid("set in/out markers before exporting"));
                    }
                    self.run_export(options)?;
                }
                UiAction::RetryExport => {
                    let blocked = self.session.transaction_active();
                    let project = self.session.project().clone();
                    self.dialogs.export.retry(project, blocked)?;
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
                UiAction::Play => self
                    .playback
                    .transport(Transport::Play, self.session.project().clone())?,
                UiAction::Pause => self
                    .playback
                    .transport(Transport::Pause, self.session.project().clone())?,
                UiAction::BackSecond => self
                    .playback
                    .transport(Transport::BackSecond, self.session.project().clone())?,
                UiAction::ForwardSecond => self
                    .playback
                    .transport(Transport::ForwardSecond, self.session.project().clone())?,
                UiAction::FrameBack => self
                    .playback
                    .transport(Transport::FrameBack, self.session.project().clone())?,
                UiAction::FrameForward => self
                    .playback
                    .transport(Transport::FrameForward, self.session.project().clone())?,
                UiAction::Start => self
                    .playback
                    .transport(Transport::Start, self.session.project().clone())?,
                UiAction::End => self
                    .playback
                    .transport(Transport::End, self.session.project().clone())?,
                UiAction::Undo | UiAction::Redo => {
                    let preference_changed = if action == UiAction::Undo {
                        self.preference_history.undo(&mut self.preferences)
                    } else {
                        self.preference_history.redo(&mut self.preferences)
                    };
                    if !preference_changed {
                        self.session.apply(if action == UiAction::Undo {
                            engine::Command::Undo {}
                        } else {
                            engine::Command::Redo {}
                        })?;
                        self.selection = self.session.selection().clone();
                        self.selection_scopes = SelectionScopes::default();
                        self.properties.clear_drafts();
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
                UiAction::Ripple => {
                    let after = !self.preferences.ripple;
                    self.dispatch(
                        PREFERENCE_COMMAND,
                        serde_json::json!({"key":"ripple", "value":after}),
                    )?;
                }
                UiAction::ZoomIn => self.zoom_timeline(ZOOM_STEP),
                UiAction::ZoomOut => self.zoom_timeline(1.0 / ZOOM_STEP),
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
                    self.desk().clear_error();
                }
                UiAction::ProjectInspector => self.properties.details = false,
                UiAction::DetailsInspector => self.properties.details = true,
                UiAction::PreviewSnapshot => self.open_snapshot_dialog()?,
                UiAction::PreviewFullscreen => self.player.fullscreen = !self.player.fullscreen,
                UiAction::PreviewFit => self.set_preview_fit()?,
                UiAction::PreviewZoomIn => self.preview_zoom_by(preview::ZOOM_STEP)?,
                UiAction::PreviewZoomOut => self.preview_zoom_by(1.0 / preview::ZOOM_STEP)?,
                UiAction::PreviewZoomPreset => {
                    let percent =
                        u32::try_from(argument.ok_or(Error::Unsupported("preview zoom"))?)
                            .map_err(|_| Error::Unsupported("preview zoom"))?;
                    let viewport = self.player.viewport;
                    let canvas = self.preview_canvas_size();
                    let zoom = preview::normalize_zoom(percent as f32 / 100.0)
                        .ok_or(Error::Invalid("preview zoom"))?;
                    self.set_preview_zoom(zoom, viewport, canvas)?;
                }
                UiAction::Seek
                | UiAction::SetPreviewZoom
                | UiAction::SetPreference
                | UiAction::SetKeybinding
                | UiAction::ResetKeybindings => {
                    return Err(Error::Unsupported("action requires parameters"));
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
