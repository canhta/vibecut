//! Routes editor actions through the facade.

use crate::editor::{
    ActionTarget, Arc, AssetId, Editor, Error, ItemId, Locale, MediaView, Preferences, Result,
    SelectionScopes, Text, UiAction, actions, media_panel,
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
        // Files on the OS clipboard win over the in-app clipboard and insert at the
        // playhead; a focused text field keeps its own paste.
        if id == "edit.paste" && !self.frame.text_focused {
            let paths = self.assets.os_files.read();
            if !paths.is_empty() {
                return self.activate_with(None, |editor| {
                    let at = editor.playback.position()?;
                    editor.with_assets(|assets, port| assets.paste_files(port, paths, at))
                });
            }
        }
        // A disabled style action must not flush an unrelated caption draft.
        if (id.starts_with("subtitles.style.")
            || id.starts_with("graphic.insert.")
            || matches!(id, "edit.copy" | "edit.cut" | "edit.paste"))
            && !self.action_enabled(id)
        {
            return Err(Error::ActionDisabled(spec.id));
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
        let flushed = if let Some(subtitles) = &mut self.assets.captions {
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
        self.remember_view()?;
        let before = self.session.project().clone();
        let result = execute(self)?;
        if id.is_none() {
            self.prune_selection();
        }
        self.assets.prune_media_selection(&self.session);
        if !Arc::ptr_eq(&before, self.session.project()) {
            self.sync_preview();
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
        // A transition has no ripple: both delete chords remove just it.
        if matches!(id, "timeline.delete" | "timeline.ripple_delete")
            && self.selection_scopes.active(self.session.selection())
                == crate::editor::selection::SelectionScope::Transition
        {
            self.delete_selected_transition();
            return Ok(());
        }
        if id == "timeline.delete" {
            match self.selection_scopes.active(self.session.selection()) {
                crate::editor::selection::SelectionScope::MaskPoints => {
                    self.delete_mask_points();
                    return Ok(());
                }
                crate::editor::selection::SelectionScope::Keyframes => {
                    let keyframes = self.selection_scopes.keyframes.clone();
                    self.delete_keyframes(&keyframes);
                    return Ok(());
                }
                _ => {}
            }
        }
        match spec.target {
            ActionTarget::Engine(route) => self.execute_engine_action(route)?,
            ActionTarget::Ui { action, .. } => match action {
                UiAction::Copy => match self.selection_scopes.active(self.session.selection()) {
                    crate::editor::selection::SelectionScope::Keyframes => {
                        let keys = self.clipboard_keys();
                        self.session
                            .apply(engine::Command::CopyKeyframes { keys })?;
                    }
                    crate::editor::selection::SelectionScope::Items => {
                        let items = self.session.selection().iter().copied().collect();
                        self.session.apply(engine::Command::CopyItems { items })?;
                    }
                    crate::editor::selection::SelectionScope::MaskPoints => {
                        let (item, mask, points) = self
                            .mask_point_group()
                            .ok_or(Error::Invalid("clipboard target"))?;
                        self.session.apply(engine::Command::CopyMaskPoints {
                            item,
                            mask,
                            points,
                        })?;
                    }
                    crate::editor::selection::SelectionScope::Transition
                    | crate::editor::selection::SelectionScope::None => {
                        return Err(Error::Invalid("clipboard scope"));
                    }
                },
                UiAction::CopyAttributes => {
                    let item = crate::editor::context::single(self.session.selection())
                        .ok_or(Error::Invalid("clipboard target"))?;
                    self.session
                        .apply(engine::Command::CopyItems { items: vec![item] })?;
                }
                UiAction::Cut => {
                    let scope = self.selection_scopes.active(self.session.selection());
                    let target = self.cut_target().ok_or(Error::Invalid("clipboard scope"))?;
                    self.session
                        .apply(engine::Command::CutSelection { target })?;
                    match scope {
                        crate::editor::selection::SelectionScope::Keyframes => {
                            self.selection_scopes.keyframes.clear();
                        }
                        // The cut anchors are gone; the item stays selected.
                        crate::editor::selection::SelectionScope::MaskPoints => {
                            self.selection_scopes.mask_points.clear();
                        }
                        _ => self.session.set_selection(Default::default())?,
                    }
                }
                UiAction::Paste => {
                    let scope = self.selection_scopes.active(self.session.selection());
                    let mut pasted_items = false;
                    let outcome = match scope {
                        crate::editor::selection::SelectionScope::Items
                            if !self.session.can_paste_items(at)
                                && crate::editor::context::single(self.session.selection())
                                    .is_some_and(|item| {
                                        self.session.can_paste_keyframes(item, at)
                                    }) =>
                        {
                            let item = crate::editor::context::single(self.session.selection())
                                .ok_or(Error::Invalid("clipboard target"))?;
                            self.session.apply(engine::Command::PasteKeyframes {
                                item,
                                at,
                                source: engine::KeyframePasteSource::Stored,
                            })?
                        }
                        crate::editor::selection::SelectionScope::Keyframes => {
                            let item = crate::editor::context::single(self.session.selection())
                                .ok_or(Error::Invalid("clipboard target"))?;
                            self.session.apply(engine::Command::PasteKeyframes {
                                item,
                                at,
                                source: engine::KeyframePasteSource::Stored,
                            })?
                        }
                        // Cut leaves nothing selected, so an empty selection pastes at the playhead.
                        crate::editor::selection::SelectionScope::Items
                        | crate::editor::selection::SelectionScope::None => {
                            pasted_items = true;
                            self.session.apply(engine::Command::PasteItems {
                                at,
                                source: engine::ItemPasteSource::Stored,
                            })?
                        }
                        crate::editor::selection::SelectionScope::MaskPoints => {
                            // The copied anchors land after the last selected one and
                            // become the point scope, so a repeated Paste chains.
                            let (item, mask, points) = self
                                .mask_point_group()
                                .ok_or(Error::Invalid("clipboard target"))?;
                            let after = points
                                .iter()
                                .copied()
                                .max()
                                .ok_or(Error::Invalid("clipboard target"))?;
                            let count = self.session.mask_point_clipboard_len();
                            let outcome = self.session.apply(engine::Command::PasteMaskPoints {
                                item,
                                mask: mask.clone(),
                                after,
                                source: engine::ItemPasteSource::Stored,
                            })?;
                            if outcome.changed {
                                self.selection_scopes.mask_points = (after + 1..=after + count)
                                    .map(|point| crate::editor::MaskPointSelection {
                                        item,
                                        mask: mask.clone(),
                                        point,
                                    })
                                    .collect();
                            }
                            return Ok(());
                        }
                        crate::editor::selection::SelectionScope::Transition => {
                            return Err(Error::Invalid("clipboard scope"));
                        }
                    };
                    if outcome.changed {
                        self.selection_scopes = SelectionScopes::default();
                        if pasted_items {
                            self.session.set_selection(
                                outcome
                                    .created
                                    .iter()
                                    .filter_map(|id| match id {
                                        engine::CreatedId::Item(item) => Some(*item),
                                        _ => None,
                                    })
                                    .collect(),
                            )?;
                        }
                    }
                }
                UiAction::MediaView => {
                    let view = match self.preferences.media_view {
                        MediaView::Grid => MediaView::List,
                        MediaView::List => MediaView::Grid,
                    };
                    self.set_preference("media_view", serde_json::to_value(view)?)?;
                }
                UiAction::MediaSort | UiAction::MediaSortBy(_) => {
                    let key = match action {
                        UiAction::MediaSortBy(key) => key,
                        _ => self.preferences.media_sort.key,
                    };
                    let next = media_panel::next_sort(self.preferences.media_sort, key);
                    self.set_preference("media_sort", serde_json::to_value(next)?)?;
                }
                UiAction::RequestDeleteMedia => {
                    let ids = self.assets.media_selection().iter().copied().collect();
                    self.assets.request_media_delete(&self.session, ids);
                }
                UiAction::CancelDeleteMedia => self.assets.cancel_media_delete(),
                UiAction::ConfirmDeleteMedia => {
                    self.with_assets(|assets, port| assets.confirm_media_delete(port))?;
                }
                UiAction::RevealMedia => {
                    let asset = AssetId(argument.ok_or(Error::Unsupported("media identity"))?);
                    self.reveal_media(asset)?;
                }
                UiAction::InsertText => self.with_assets(|assets, port| assets.insert_text(port)),
                UiAction::InsertGraphic(shape) => {
                    self.with_assets(|assets, port| assets.insert_graphic(port, shape));
                }
                UiAction::Subtitles | UiAction::SubtitleCue => {
                    self.prepare_subtitles()?;
                    let mut options = self.preferences.subtitles.clone();
                    let selected = self.session.selection().iter().next().copied();
                    if let Some(panel) = &mut self.assets.captions {
                        panel.activate(&mut self.session, &mut options, selected, at, id)?;
                        let requests = self.assets.caption_requests();
                        self.requests.extend(requests);
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
                        self.session.set_selection(Default::default())?;
                        self.selection_scopes = SelectionScopes::default();
                    }
                }
                UiAction::Export => self.open_export()?,
                UiAction::Crop => self.with_player(|player, port| {
                    crate::editor::panels::player::crop::open(player, port)
                })?,
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
                    self.set_preference("export", serde_json::to_value(options)?)?;
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
                    self.lifecycle_action = Some(crate::services::lifecycle::EditorAction::Save)
                }
                UiAction::SaveAs => {
                    self.lifecycle_action = Some(crate::services::lifecycle::EditorAction::SaveAs)
                }
                UiAction::FocusHome => {
                    self.lifecycle_action =
                        Some(crate::services::lifecycle::EditorAction::FocusHome)
                }
                UiAction::ExitProject => {
                    self.lifecycle_action =
                        Some(crate::services::lifecycle::EditorAction::ExitProject)
                }
                UiAction::OpenHelp => {
                    self.lifecycle_action = Some(crate::services::lifecycle::EditorAction::OpenHelp)
                }
                UiAction::OpenFeedback => {
                    self.lifecycle_action =
                        Some(crate::services::lifecycle::EditorAction::OpenFeedback)
                }
                UiAction::DeleteProject => self.frame.delete_open = true,
                UiAction::CancelDeleteProject => self.frame.delete_open = false,
                UiAction::ConfirmDeleteProject => {
                    self.frame.delete_open = false;
                    self.lifecycle_action =
                        Some(crate::services::lifecycle::EditorAction::DeleteProject);
                }
                UiAction::RenameProject => {
                    self.frame.rename =
                        Some(self.session.project().data().name.as_str().to_owned());
                    self.frame.rename_focus = true;
                }
                UiAction::Import => {
                    if self.importing() {
                        self.assets.import_notice = Some(self.tr(Text::ImportBusy).to_owned());
                    } else {
                        self.lifecycle_action =
                            Some(crate::services::lifecycle::EditorAction::ImportMedia);
                    }
                }
                UiAction::CancelImport => {
                    if let Some(queue) = &self.assets.imports {
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
                        self.frame.preference_history.undo(&mut self.preferences)
                    } else {
                        self.frame.preference_history.redo(&mut self.preferences)
                    };
                    if !preference_changed {
                        self.session.apply(if action == UiAction::Undo {
                            engine::Command::Undo {}
                        } else {
                            engine::Command::Redo {}
                        })?;
                        // A selected transition survives history while it still exists.
                        let transition = self.selection_scopes.transition;
                        self.selection_scopes = SelectionScopes::default();
                        self.selection_scopes.transition = transition;
                        self.forget_missing_transition();
                        if self.selection_scopes.transition.is_some() {
                            self.session.set_selection(Default::default())?;
                        }
                        self.properties.clear_drafts();
                    } else {
                        self.sync_keymap();
                    }
                }
                UiAction::SelectAll => self.select_all(),
                UiAction::DeselectAll => {
                    self.replace_selection(Default::default());
                }
                UiAction::Escape => self.cancel_interaction(),
                UiAction::Snap => {
                    let after = !self.preferences.snapping;
                    self.set_preference("snapping", serde_json::json!(after))?;
                }
                UiAction::Ripple => {
                    let after = !self.preferences.ripple;
                    self.set_preference("ripple", serde_json::json!(after))?;
                }
                UiAction::Loop => {
                    let after = !self.preferences.playback_loop;
                    self.set_preference("playback_loop", serde_json::json!(after))?;
                    self.sync_loop()?;
                }
                UiAction::PreviewAxis => {
                    let after = !self.preferences.preview_axis;
                    self.set_preference("preview_axis", serde_json::json!(after))?;
                }
                UiAction::SelectTool => {
                    self.timeline
                        .set_tool(crate::editor::timeline::Tool::Select);
                }
                UiAction::SplitTool => {
                    self.timeline.set_tool(crate::editor::timeline::Tool::Split);
                }
                UiAction::SelectLeftward => self.select_by_playhead(false)?,
                UiAction::SelectRightward => self.select_by_playhead(true)?,
                UiAction::ZoomIn => self.zoom_timeline(true),
                UiAction::ZoomOut => self.zoom_timeline(false),
                UiAction::Theme => {
                    let after = !self.preferences.dark;
                    self.set_preference("dark", serde_json::json!(after))?;
                }
                UiAction::Locale => {
                    let after = if self.preferences.locale == Locale::En {
                        Locale::Vi
                    } else {
                        Locale::En
                    };
                    self.set_preference("locale", serde_json::to_value(after)?)?;
                }
                UiAction::ResetLayout => {
                    self.set_preference(
                        "panels",
                        serde_json::to_value(Preferences::default().panels)?,
                    )?;
                }
                UiAction::OpenPreferences => self.dialogs.preferences.open = true,
                UiAction::OpenModels => self.open_models()?,
                UiAction::OpenVoiceover => self.dialogs.voiceover.open(),
                UiAction::OpenShortcuts => self.open_shortcuts(),
                UiAction::RetryPreview => {
                    self.playback.clear()?;
                    self.error = None;
                }
                UiAction::ProjectInspector => self.properties.set_details(false),
                UiAction::DetailsInspector => self.properties.set_details(true),
                UiAction::PreviewSnapshot => self.open_snapshot_dialog()?,
                UiAction::PreviewCopySnapshot => self.copy_snapshot_requested = true,
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
                UiAction::ImportKeybindings => {
                    self.lifecycle_action = Some(
                        crate::services::lifecycle::EditorAction::KeybindingsImport {
                            replace: false,
                        },
                    );
                }
                UiAction::ExportKeybindings => {
                    self.lifecycle_action =
                        Some(crate::services::lifecycle::EditorAction::KeybindingsExport);
                }
                UiAction::JumpBookmark(direction) => {
                    self.dispatch(crate::editor::AppCommand::JumpBookmark(direction))?;
                }
                UiAction::Seek
                | UiAction::SetPreviewZoom
                | UiAction::SetPreference
                | UiAction::SetFavorite
                | UiAction::ListFavorites
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
                        return Err(Error::Engine(engine::control::Error::Edit(
                            engine::EditError::NoItem(item),
                        )));
                    }
                    self.select_single(item);
                }
            },
        }
        Ok(())
    }
}
