//! Adapts editor actions to engine commands.

use crate::editor::{
    ActionTarget, AssetId, Command, CreatedId, Editor, EngineAction, Error, ItemId, Result, Text,
    Tick, UiAction, actions,
};
use crate::i18n;
use crate::services::preview;

/// One app-owned command the frame applies; none of them mutate project history.
#[derive(Clone, Debug, PartialEq)]
pub enum AppCommand {
    /// `export.run`: start the export sheet with typed caller settings.
    Export(crate::export::ExportOptions),
    /// `playback.seek`: move the shared transport to an absolute tick.
    Seek(Tick),
    /// `preview.set_zoom`: fit the frame or set an explicit scale.
    PreviewZoom(PreviewZoom),
    /// `ui.keybindings.set`: replace one action's chords.
    SetKeybinding { action: String, chords: Vec<String> },
    /// `ui.keybindings.reset`: reset one action's chords, or every override.
    ResetKeybindings { action: Option<String> },
    /// `media.reveal`: reveal one library asset.
    RevealMedia(AssetId),
}

/// The two forms of `preview.set_zoom`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PreviewZoom {
    Fit,
    Zoom(f32),
}

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
    /// App command dispatcher shared by UI and agents. Every variant is typed;
    /// preference edits have their own entry and never touch engine history.
    pub fn dispatch(&mut self, command: AppCommand) -> Result<()> {
        match command {
            AppCommand::Export(options) => self.run_export(options),
            AppCommand::Seek(at) => {
                let at = at.clamp(Tick::ZERO, self.session.project().duration());
                self.playback
                    .seek(at, self.playback.playing(), self.session.project().clone())?;
                self.remember_view()
            }
            AppCommand::PreviewZoom(PreviewZoom::Fit) => self.set_preview_fit(),
            AppCommand::PreviewZoom(PreviewZoom::Zoom(zoom)) => {
                let viewport = self.player.viewport;
                let canvas = self.preview_canvas_size();
                let zoom = preview::normalize_zoom(zoom).ok_or(Error::Invalid("preview zoom"))?;
                self.set_preview_zoom(zoom, viewport, canvas)
            }
            AppCommand::SetKeybinding { action, chords } => self.set_keybinding(&action, chords),
            AppCommand::ResetKeybindings { action } => match action {
                Some(action) => self.reset_keybinding(&action),
                None => self.reset_keybindings(),
            },
            AppCommand::RevealMedia(asset) => self.activate(&format!("media.reveal.{}", asset.0)),
        }
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
                self.frame.preference_history.can_undo()
                    || engine::CommandKind::Undo.enabled(&self.session)
            }
            ActionTarget::Ui {
                action: UiAction::Redo,
                ..
            } => {
                self.frame.preference_history.can_redo()
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
