//! Adapts editor actions to engine commands.

use crate::editor::{
    ActionTarget, Command, CreatedId, Editor, EngineAction, Error, ItemId, PREFERENCE_COMMAND,
    Result, Text, Tick, UiAction, actions,
};
use crate::i18n;
use crate::services::preview;

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
        self.desk().act(id);
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
            ActionTarget::Ui {
                action: UiAction::Crop,
                ..
            } => crate::editor::panels::player::crop::available(self),
            ActionTarget::Ui { .. } => true,
        }
    }

    pub(in crate::editor) fn engine_commands(&self, route: EngineAction) -> Result<Vec<Command>> {
        Ok(match route {
            EngineAction::Split => {
                let at = self.playback.position()?;
                self.split_targets()
                    .into_iter()
                    .map(|item| Command::SplitAt { item, at })
                    .collect()
            }
            EngineAction::SplitLeft | EngineAction::SplitRight => {
                let at = self.playback.position()?;
                let items = self.split_targets();
                if items.is_empty() {
                    Vec::new()
                } else if route == EngineAction::SplitLeft {
                    vec![Command::TimelineSplitLeft { items, at }]
                } else {
                    vec![Command::TimelineSplitRight { items, at }]
                }
            }
            EngineAction::Duplicate => vec![Command::DuplicateItems {
                items: self.duplicate_targets(),
            }],
            EngineAction::SetHidden => {
                let items = self.visual_targets();
                let hidden = items.iter().any(|item| !self.item_hidden(*item));
                vec![Command::SetItemsHidden { items, hidden }]
            }
            EngineAction::SetMuted => {
                let items = self.audio_targets();
                let muted = items.iter().any(|item| !self.item_muted(*item));
                vec![Command::SetItemsMuted { items, muted }]
            }
            EngineAction::ExtractAudio => vec![Command::ExtractAudio {
                items: self.extract_targets(),
            }],
            EngineAction::RecoverAudio => vec![Command::RecoverAudio {
                items: self.recover_targets(),
            }],
            EngineAction::Delete | EngineAction::RippleDelete => {
                if route.command(self.preferences.ripple) == engine::CommandKind::RippleDelete {
                    self.selection_by_end(true)
                        .into_iter()
                        .map(|item| Command::RippleDelete { item })
                        .collect()
                } else {
                    vec![Command::Delete {
                        items: self.selection.iter().copied().collect(),
                    }]
                }
            }
            EngineAction::InsertMedia => Vec::new(),
            EngineAction::AddTrack => vec![crate::editor::timeline::add_track_command(
                &self.session,
                &self.selection,
                self.preferences.locale,
            )],
            EngineAction::ToggleBookmark => vec![Command::ToggleBookmark {
                at: self.playback.position()?,
            }],
            EngineAction::CreateScene => {
                let count = self.session.project().data().scenes.iter().count();
                let number = i18n::number(self.preferences.locale, (count + 1) as u64);
                let name = i18n::format(
                    self.preferences.locale,
                    Text::SceneDefaultName,
                    &[("n", &number)],
                );
                vec![Command::CreateScene {
                    name: engine::NfcText::from(name),
                }]
            }
            EngineAction::DeleteScenes => vec![Command::DeleteScenes {
                scenes: self.timeline.scenes.selected().iter().copied().collect(),
            }],
        })
    }

    pub(in crate::editor) fn execute_engine_action(&mut self, route: EngineAction) -> Result<()> {
        if route == EngineAction::InsertMedia {
            self.insert_selected_assets();
            return Ok(());
        }
        let commands = self.engine_commands(route)?;
        if commands.is_empty() {
            return Ok(());
        }
        let created = self
            .session
            .apply_with_ripple(Command::Batch { commands }, self.preferences.ripple)?
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
            | EngineAction::RecoverAudio
            | EngineAction::ToggleBookmark => self.prune_selection(),
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
