//! Adapts editor actions to engine commands.

use crate::editor::{
    ActionTarget, Command, CreatedId, Editor, EngineAction, Error, ItemId, PREFERENCE_COMMAND,
    Result, Text, Tick, UiAction, actions, catalog, preview,
};

impl Editor {
    fn split_targets(&self) -> Vec<ItemId> {
        let at = self.playback.position().unwrap_or(Tick::ZERO);
        self.session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| {
                let locked = track.locked;
                track.items.iter().map(move |item| (locked, item))
            })
            .filter(|(locked, item)| {
                !locked
                    && item.start < at
                    && item.end().is_ok_and(|end| at < end)
                    && (self.selection.is_empty() || self.selection.contains(&item.id))
            })
            .map(|(_, item)| item.id)
            .collect()
    }

    fn selected_items(&self) -> Vec<(bool, &project::Item)> {
        self.session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| {
                let locked = track.locked;
                track.items.iter().map(move |item| (locked, item))
            })
            .filter(|(_, item)| self.selection.contains(&item.id))
            .collect()
    }

    fn visual_targets(&self) -> Vec<ItemId> {
        self.selected_items()
            .into_iter()
            .filter(|(locked, item)| {
                !locked && !matches!(item.content, project::ItemParams::Audio(_))
            })
            .map(|(_, item)| item.id)
            .collect()
    }

    fn audio_targets(&self) -> Vec<ItemId> {
        self.selected_items()
            .into_iter()
            .filter(|(locked, item)| {
                !locked
                    && matches!(
                        item.content,
                        project::ItemParams::Audio(_) | project::ItemParams::Video(_)
                    )
            })
            .map(|(_, item)| item.id)
            .collect()
    }

    fn extract_targets(&self) -> Vec<ItemId> {
        self.selected_items()
            .into_iter()
            .filter(|(locked, item)| {
                !locked
                    && matches!(
                        &item.content,
                        project::ItemParams::Video(params) if params.source_audio_enabled
                    )
            })
            .map(|(_, item)| item.id)
            .collect()
    }

    fn recover_targets(&self) -> Vec<ItemId> {
        self.selected_items()
            .into_iter()
            .filter(|(locked, item)| {
                !locked
                    && matches!(
                        &item.content,
                        project::ItemParams::Video(params) if !params.source_audio_enabled
                    )
            })
            .map(|(_, item)| item.id)
            .collect()
    }

    fn duplicate_targets(&self) -> Vec<ItemId> {
        self.selected_items()
            .into_iter()
            .filter(|(locked, _)| !locked)
            .map(|(_, item)| item.id)
            .collect()
    }

    fn item_hidden(&self, item: ItemId) -> bool {
        self.session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)
            .is_some_and(|item| item.hidden)
    }

    fn item_muted(&self, item: ItemId) -> bool {
        self.session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)
            .is_some_and(|item| match &item.content {
                project::ItemParams::Video(params) => params.muted,
                project::ItemParams::Audio(params) => params.muted,
                _ => false,
            })
    }

    pub(in crate::editor) fn action(&mut self, id: &str) {
        if let Err(error) = self.activate(id) {
            self.error = Some(error.to_string());
        }
    }
    /// App action dispatcher shared by UI and agents. Preference edits use
    /// `ui.prefs.set` with `{"key": ..., "value": ...}` typed params.
    pub fn dispatch(&mut self, id: &str, params: serde_json::Value) -> Result<()> {
        if id == "export.run" {
            let options: crate::export::ExportOptions = serde_json::from_value(params)?;
            return self.run_export(options);
        }
        if id == "playback.seek" {
            let time = params
                .get("time")
                .and_then(serde_json::Value::as_i64)
                .ok_or(Error::Invalid("seek time"))?;
            let at = Tick(time).clamp(Tick::ZERO, self.session.project().duration());
            self.playback
                .seek(at, self.playback.playing(), self.session.project().clone())?;
            self.remember_view()?;
            return Ok(());
        }
        if id == "preview.set_zoom" {
            let viewport = self.player.viewport;
            let canvas = self.preview_canvas_size();
            if params
                .get("fit")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                self.set_preview_fit()?;
            } else {
                let zoom = params
                    .get("zoom")
                    .and_then(serde_json::Value::as_f64)
                    .ok_or(Error::Invalid("preview zoom"))?;
                let zoom =
                    preview::normalize_zoom(zoom as f32).ok_or(Error::Invalid("preview zoom"))?;
                self.set_preview_zoom(zoom, viewport, canvas)?;
            }
            return Ok(());
        }
        if id == actions::KEYBINDINGS_SET_COMMAND {
            let action = params
                .get("action")
                .and_then(serde_json::Value::as_str)
                .ok_or(Error::Invalid("keybinding action"))?;
            let chords = params
                .get("bindings")
                .cloned()
                .ok_or(Error::Invalid("keybinding chords"))
                .and_then(|value| {
                    serde_json::from_value::<Vec<String>>(value)
                        .map_err(|_| Error::Invalid("keybinding chords"))
                })?;
            return self.set_keybinding(action, chords);
        }
        if id == actions::KEYBINDINGS_RESET_COMMAND {
            let action = params.get("action").and_then(serde_json::Value::as_str);
            return if let Some(action) = action {
                self.reset_keybinding(action)
            } else {
                self.reset_keybindings()
            };
        }
        if id == PREFERENCE_COMMAND {
            let key = params
                .get("key")
                .and_then(serde_json::Value::as_str)
                .ok_or(Error::Invalid("preference key"))?;
            let value = params
                .get("value")
                .cloned()
                .ok_or(Error::Invalid("preference value"))?;
            return self.set_preference(key, value);
        }
        if id == "media.reveal" {
            let asset = params
                .get("asset")
                .and_then(serde_json::Value::as_u64)
                .ok_or(Error::Invalid("media identity"))?;
            return self.activate(&format!("media.reveal.{asset}"));
        }
        self.activate(id)
    }
    /// Availability is shared by widget admission and direct/key activation.
    /// Engine predicates remain engine-owned; selection and app workflows are local.
    pub fn action_enabled(&self, id: &str) -> bool {
        let Some((spec, _)) = actions::resolve(id) else {
            return false;
        };
        match spec.target {
            ActionTarget::Pending { .. } => false,
            ActionTarget::Engine(route) => {
                route
                    .command(self.preferences.ripple)
                    .enabled(&self.session)
                    && match route {
                        EngineAction::Duplicate => !self.duplicate_targets().is_empty(),
                        EngineAction::SetHidden => !self.visual_targets().is_empty(),
                        EngineAction::SetMuted => !self.audio_targets().is_empty(),
                        EngineAction::ExtractAudio => !self.extract_targets().is_empty(),
                        EngineAction::RecoverAudio => !self.recover_targets().is_empty(),
                        EngineAction::Delete | EngineAction::RippleDelete => {
                            !self.selection.is_empty()
                                || self.selection_scopes.active(&self.selection)
                                    == crate::selection::SelectionScope::Keyframes
                        }
                        EngineAction::DeleteScenes => !self.timeline.scenes.selected().is_empty(),
                        EngineAction::InsertMedia => {
                            !self.importing() && !self.media_selection.is_empty()
                        }
                        _ => true,
                    }
            }
            ActionTarget::Ui {
                action: UiAction::Undo,
                ..
            } => {
                self.preference_history.can_undo()
                    || engine::CommandKind::Undo.enabled(&self.session)
            }
            ActionTarget::Ui {
                action: UiAction::Redo,
                ..
            } => {
                self.preference_history.can_redo()
                    || engine::CommandKind::Redo.enabled(&self.session)
            }
            ActionTarget::Ui { .. } => true,
        }
    }

    pub(in crate::editor) fn engine_action_params(
        &self,
        route: EngineAction,
    ) -> Result<Vec<serde_json::Value>> {
        Ok(match route {
            EngineAction::Split => {
                let at = self.playback.position()?;
                self.split_targets()
                    .into_iter()
                    .map(|item| serde_json::json!({"item":item,"at":at}))
                    .collect()
            }
            EngineAction::SplitLeft | EngineAction::SplitRight => {
                let at = self.playback.position()?;
                let targets = self.split_targets();
                if targets.is_empty() {
                    Vec::new()
                } else {
                    vec![serde_json::json!({"items":targets,"at":at})]
                }
            }
            EngineAction::Duplicate => vec![serde_json::json!({"items":self.duplicate_targets()})],
            EngineAction::SetHidden => {
                let targets = self.visual_targets();
                let hidden = targets.iter().any(|item| !self.item_hidden(*item));
                vec![serde_json::json!({"items":targets,"hidden":hidden})]
            }
            EngineAction::SetMuted => {
                let targets = self.audio_targets();
                let muted = targets.iter().any(|item| !self.item_muted(*item));
                vec![serde_json::json!({"items":targets,"muted":muted})]
            }
            EngineAction::ExtractAudio => {
                vec![serde_json::json!({"items":self.extract_targets()})]
            }
            EngineAction::RecoverAudio => {
                vec![serde_json::json!({"items":self.recover_targets()})]
            }
            EngineAction::Delete | EngineAction::RippleDelete => {
                if route.command(self.preferences.ripple) == engine::CommandKind::RippleDelete {
                    self.selection_by_end(true)
                        .into_iter()
                        .map(|item| serde_json::json!({"item":item}))
                        .collect()
                } else {
                    vec![serde_json::json!({"items":self.selection})]
                }
            }
            EngineAction::InsertMedia => Vec::new(), // Placement depends on each preceding outcome.
            EngineAction::AddTrack => vec![crate::editor::timeline::add_track_params(
                &self.session,
                &self.selection,
                self.preferences.locale,
            )],
            EngineAction::CreateScene => {
                let count = self.session.project().data().scenes.iter().count();
                let number = catalog::number(self.preferences.locale, (count + 1) as u64);
                let name = catalog::format(
                    self.preferences.locale,
                    Text::SceneDefaultName,
                    &[("n", &number)],
                );
                vec![serde_json::json!({"name":name})]
            }
            EngineAction::DeleteScenes => {
                vec![serde_json::json!({"scenes":self.timeline.scenes.selected()})]
            }
        })
    }

    pub(in crate::editor) fn execute_engine_action(&mut self, route: EngineAction) -> Result<()> {
        let command = route.command(self.preferences.ripple);
        if route == EngineAction::InsertMedia {
            self.insert_selected_assets(command);
            return Ok(());
        }
        let params = self.engine_action_params(route)?;
        if params.is_empty() {
            return Ok(());
        }
        let commands: Vec<_> = params
            .into_iter()
            .map(|params| serde_json::json!({"command":command.id(), "params":params}))
            .collect();
        let created = self
            .session
            .execute_with_ripple(
                "batch",
                serde_json::json!({"commands":commands}),
                self.preferences.ripple,
            )?
            .created;
        let created_items: Vec<ItemId> = created
            .iter()
            .filter_map(|id| match id {
                CreatedId::Item(item) => Some(*item),
                _ => None,
            })
            .collect();
        match route {
            EngineAction::Split | EngineAction::SplitLeft | EngineAction::Duplicate => {
                self.selection = created_items.iter().copied().collect();
            }
            EngineAction::Delete | EngineAction::RippleDelete => self.selection.clear(),
            EngineAction::CreateScene => {
                if let Some(CreatedId::Scene(scene)) = created.first().copied() {
                    self.cancel_interaction();
                    self.session.apply(Command::SwitchScene { scene })?;
                    self.clear_scene_selection();
                    self.timeline.scenes.end_rename();
                }
            }
            EngineAction::DeleteScenes => self.clear_scene_selection(),
            EngineAction::AddTrack => self.prune_selection(),
            EngineAction::InsertMedia => {}
            EngineAction::SplitRight
            | EngineAction::SetHidden
            | EngineAction::SetMuted
            | EngineAction::ExtractAudio
            | EngineAction::RecoverAudio => self.prune_selection(),
        }
        if route == EngineAction::SplitLeft
            && self.preferences.ripple
            && let Some(start) = created_items
                .iter()
                .filter_map(|id| {
                    self.session
                        .project()
                        .data()
                        .scene()
                        .timeline
                        .iter()
                        .flat_map(|track| &track.items)
                        .find(|item| item.id == *id)
                        .map(|item| item.start)
                })
                .min()
        {
            self.seek(start)?;
        }
        self.session.end_merge();
        Ok(())
    }
}
