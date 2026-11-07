//! Adapts editor actions to engine commands.

use crate::editor::{
    ActionTarget, Command, CreatedId, Editor, EngineAction, Error, PREFERENCE_COMMAND, Result,
    Text, Tick, UiAction, actions, preview,
};
use crate::i18n;

impl Editor {
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

    pub(in crate::editor) fn engine_commands(&self, route: EngineAction) -> Result<Vec<Command>> {
        Ok(match route {
            EngineAction::Split => {
                let at = self.playback.position()?;
                self.session
                    .project()
                    .data()
                    .scene()
                    .timeline
                    .iter()
                    .flat_map(|track| &track.items)
                    .filter(|item| {
                        item.start < at
                            && item.end().is_ok_and(|end| at < end)
                            && (self.selection.is_empty() || self.selection.contains(&item.id))
                    })
                    .map(|item| Command::SplitAt { item: item.id, at })
                    .collect()
            }
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
        let command = route.command(self.preferences.ripple);
        if route == EngineAction::InsertMedia {
            self.insert_selected_assets(command);
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
        match route {
            EngineAction::Split => {
                self.selection = created
                    .into_iter()
                    .filter_map(|id| match id {
                        CreatedId::Item(item) => Some(item),
                        _ => None,
                    })
                    .collect();
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
        }
        self.session.end_merge();
        Ok(())
    }
}
