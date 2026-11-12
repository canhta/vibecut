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
    /// `playback.jump_bookmark`: seek to the nearest eligible bookmark in one
    /// direction, without wrapping or mutating the project.
    JumpBookmark(actions::BookmarkDirection),
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
    pub(in crate::editor) fn caption_bank_ready(&self) -> bool {
        self.preferences.assets_tab == crate::panels::AssetsTab::Captions
            && self.properties.input_ready(&self.session)
            && !self.player.transform.text_edit_open()
            && !matches!(&self.session, crate::lifecycle::EditorSession::Persistent(s) if s.saving())
            && !self.importing()
            && self.lifecycle_action.is_none()
            && !self.overlay_open
            && !self.dialogs.export.is_open()
            && !self.dialogs.models.is_open()
    }
    pub(in crate::editor) fn clipboard_keys(&self) -> Vec<engine::KeyframeRef> {
        self.selection_scopes
            .keyframes
            .iter()
            .map(|k| engine::KeyframeRef {
                item: k.item,
                path: k.path.clone(),
                component: k.component,
                time: k.time,
            })
            .collect()
    }
    fn clipboard_available(&self, copy: bool) -> bool {
        if !self.properties.input_ready(&self.session)
            || self.session.transaction_active()
            || self.player.transform.text_edit_open()
            || self.player.timecode_edit.is_some()
            || self.frame.rename.is_some()
            || self.timeline.renaming_track()
            || self.timeline.scenes.is_open()
            || self.timeline.bookmark_popover().is_some()
            || self.frame.recording_active
            || self.frame.shortcuts_open
            || self
                .subtitles
                .as_ref()
                .is_some_and(|s| !s.edit_ready(&self.session))
            || matches!(&self.session, crate::lifecycle::EditorSession::Persistent(s) if s.saving())
            || self.importing()
            || self.lifecycle_action.is_some()
            || self.overlay_open
            || self.dialogs.export.is_open()
            || self.dialogs.models.is_open()
        {
            return false;
        }
        use crate::selection::SelectionScope;
        let scope = self.selection_scopes.active(&self.selection);
        if copy {
            scope == SelectionScope::Keyframes
                && self.session.can_copy_keyframes(&self.clipboard_keys())
        } else {
            matches!(scope, SelectionScope::Items | SelectionScope::Keyframes)
                && crate::editor::context::single(&self.selection).is_some_and(|item| {
                    self.playback
                        .position()
                        .is_ok_and(|at| self.session.can_paste_keyframes(item, at))
                })
        }
    }

    pub(in crate::editor) fn graphic_insertion_available(&self) -> bool {
        self.properties.input_ready(&self.session)
            && !self.player.transform.text_edit_open()
            && self
                .subtitles
                .as_ref()
                .is_none_or(|s| s.edit_ready(&self.session))
            && !matches!(&self.session, crate::lifecycle::EditorSession::Persistent(s) if s.saving())
            && !self.importing()
            && self.lifecycle_action.is_none()
            && !self.overlay_open
            && !self.dialogs.export.is_open()
            && !self.dialogs.models.is_open()
    }

    pub(in crate::editor) fn text_animation_available(
        &self,
        state: &crate::editor::properties::State,
    ) -> bool {
        state
            .animation_target(&self.session, &self.selection)
            .is_some()
            && !self.player.transform.text_edit_open()
            && self
                .subtitles
                .as_ref()
                .is_none_or(|s| s.edit_ready(&self.session))
            && !matches!(&self.session, crate::lifecycle::EditorSession::Persistent(s) if s.saving())
            && !self.importing()
            && self.lifecycle_action.is_none()
            && !self.overlay_open
            && !self.dialogs.export.is_open()
            && !self.dialogs.models.is_open()
    }
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
            AppCommand::JumpBookmark(direction) => {
                let at = self.playback.position()?;
                match actions::bookmark_target(self.session.project().data().scene(), at, direction)
                {
                    Some(target) => self.dispatch(AppCommand::Seek(target)),
                    None => Ok(()),
                }
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
            ActionTarget::Ui {
                action: UiAction::InsertGraphic(_),
                ..
            } => self.graphic_insertion_available(),
            ActionTarget::Ui {
                action: UiAction::CopyKeyframes,
                ..
            } => self.clipboard_available(true),
            ActionTarget::Ui {
                action: UiAction::PasteKeyframes,
                ..
            } => self.clipboard_available(false),
            ActionTarget::Ui {
                action: UiAction::ApplyTextAnimation(_),
                ..
            } => self.text_animation_available(&self.properties),
            ActionTarget::Ui {
                action: UiAction::Subtitles,
                ..
            } if id.starts_with("subtitles.style.") => {
                self.subtitles.as_ref().is_some_and(|panel| {
                    if matches!(id, "subtitles.style.save" | "subtitles.style.manage") {
                        self.caption_bank_ready()
                            && panel.bank_available(&self.session, id == "subtitles.style.save")
                    } else {
                        (id != "subtitles.style.word_highlight" || self.caption_bank_ready())
                            && panel.style_available(&self.session)
                    }
                })
            }
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
            ActionTarget::Ui {
                action: UiAction::JumpBookmark(direction),
                ..
            } => self.playback.position().is_ok_and(|at| {
                actions::bookmark_target(self.session.project().data().scene(), at, direction)
                    .is_some()
            }),
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use actions::BookmarkDirection::{Next, Previous};
    use project::{Bookmark, BookmarkId, Color};
    use time::TICKS_PER_SECOND;

    fn mark(id: u64, second: i64) -> Bookmark {
        Bookmark {
            id: BookmarkId(id),
            start: Tick(second * TICKS_PER_SECOND),
            duration: Tick::ZERO,
            note: "Mark".into(),
            color: Color([255, 255, 0, 255]),
        }
    }

    fn at(at: i64, direction: actions::BookmarkDirection, marks: &[Bookmark]) -> Option<i64> {
        actions::bookmark_target_in(
            marks,
            Tick(10 * TICKS_PER_SECOND),
            Tick(at * TICKS_PER_SECOND),
            direction,
        )
        .map(|tick| tick.0 / TICKS_PER_SECOND)
    }

    #[test]
    fn bookmark_jump_stops_before_between_and_after_without_wrapping() {
        assert_eq!(at(0, Next, &[]), None);
        assert_eq!(at(0, Previous, &[]), None);
        let marks = [mark(1, 3), mark(2, 8)];
        assert_eq!(at(0, Next, &marks), Some(3));
        assert_eq!(at(5, Next, &marks), Some(8));
        assert_eq!(at(5, Previous, &marks), Some(3));
        assert_eq!(at(9, Next, &marks), None, "no wrap to the first mark");
        assert_eq!(at(9, Previous, &marks), Some(8));
        assert_eq!(at(3, Previous, &marks), None, "no wrap to the last mark");
        assert_eq!(at(3, Next, &marks), Some(8), "exact playhead is skipped");
        assert_eq!(at(8, Next, &marks), None);
    }

    #[test]
    fn equal_time_marks_are_ordered_and_skipped_at_their_instant() {
        let forward = [mark(1, 5), mark(9, 5), mark(4, 8)];
        let reverse = [mark(4, 8), mark(9, 5), mark(1, 5)];
        for marks in [forward, reverse] {
            assert_eq!(at(4, Next, &marks), Some(5));
            assert_eq!(
                at(5, Next, &marks),
                Some(8),
                "the shared instant is skipped"
            );
            assert_eq!(at(5, Previous, &marks), None);
            assert_eq!(at(6, Previous, &marks), Some(5));
            assert_eq!(at(7, Next, &marks), Some(8));
        }
    }

    #[test]
    fn marks_past_the_end_clamp_to_duration_once_and_zero_duration_never_moves() {
        let far = [mark(1, 15)];
        assert_eq!(at(0, Next, &far), Some(10));
        assert_eq!(at(5, Next, &far), Some(10));
        assert_eq!(at(10, Next, &far), None, "already at duration");
        assert_eq!(at(10, Previous, &far), None);
        let zero = [mark(1, 0)];
        assert_eq!(
            actions::bookmark_target_in(&zero, Tick::ZERO, Tick::ZERO, Next),
            None
        );
        assert_eq!(
            actions::bookmark_target_in(&zero, Tick::ZERO, Tick::ZERO, Previous),
            None
        );
        assert_eq!(
            actions::bookmark_target_in(&[mark(2, 15)], Tick::ZERO, Tick::ZERO, Next),
            None
        );
    }
}
