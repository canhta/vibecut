//! Maintains editor selection scope.

use crate::editor::{BTreeSet, Editor, Error, ItemId, Result, SelectionScopes, Tick, egui};

impl Editor {
    /// The selected items; the engine session is their only owner.
    pub fn selection(&self) -> &BTreeSet<ItemId> {
        self.session.selection()
    }
    /// Replace the item selection from outside a pointer interaction (agents,
    /// tests): the same write a click makes, with the open gestures the change
    /// invalidates ended first.
    pub fn select(&mut self, items: BTreeSet<ItemId>) -> Result<()> {
        if self.write_selection(items) {
            Ok(())
        } else {
            Err(Error::Invalid("selection"))
        }
    }
    /// The one item-selection write. The session refuses a change while a
    /// gesture is open, so the gestures this change invalidates end first: the
    /// inspector's scrub, wheel, curve and picker (as its per-frame
    /// reconciliation would) and the timeline's graph popover. Any other open
    /// gesture refuses the write and the error line says why.
    pub(in crate::editor) fn write_selection(&mut self, items: BTreeSet<ItemId>) -> bool {
        if self.session.selection() == &items {
            return true;
        }
        let actions = self.properties.reconcile(
            self.session.project(),
            crate::editor::context::single(&items),
        );
        self.apply_properties(actions);
        if self.timeline.graph_popover_open() {
            self.with_timeline(crate::editor::timeline::release_graph);
        }
        match self.session.set_selection(items) {
            Ok(()) => true,
            Err(error) => {
                self.fail(error.to_string());
                false
            }
        }
    }
    /// Replace the item selection: end the open scrub, drop every nested scope
    /// and stale draft, then write once.
    pub(in crate::editor) fn replace_selection(&mut self, items: BTreeSet<ItemId>) -> bool {
        self.selection_changed();
        let single = (items.len() == 1)
            .then(|| items.iter().next().copied())
            .flatten();
        let written = self.write_selection(items);
        // One selected caption item becomes the Captions panel's item, so its cue keywords
        // show in the item's Text tab. A non-caption item leaves the panel's item alone.
        if let (Some(item), Some(captions)) = (single, self.assets.captions.as_mut())
            && captions.item != Some(item)
        {
            let _ = captions.select_item(&self.session, item);
        }
        written
    }
    pub(in crate::editor) fn select_single(&mut self, item: ItemId) {
        self.replace_selection(BTreeSet::from([item]));
    }
    pub(in crate::editor) fn select_item(&mut self, item: ItemId, modifiers: egui::Modifiers) {
        let mut items = self.session.selection().clone();
        crate::editor::selection::click(
            &mut items,
            &mut None,
            item,
            modifiers.command || modifiers.ctrl || modifiers.shift,
            None,
        );
        self.replace_selection(items);
    }
    pub(in crate::editor) fn select_all(&mut self) {
        let items = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter().map(|item| item.id))
            .collect();
        self.replace_selection(items);
    }
    /// Select every unlocked item that starts before (`rightward` false) or ends
    /// after (`rightward` true) the playhead. Nothing to select leaves the selection.
    pub(in crate::editor) fn select_by_playhead(&mut self, rightward: bool) -> Result<()> {
        let at = self.playback.position()?;
        let items: BTreeSet<ItemId> = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .filter(|track| !track.locked)
            .flat_map(|track| &track.items)
            .filter(|item| {
                if rightward {
                    item.end().is_ok_and(|end| end > at)
                } else {
                    item.start < at
                }
            })
            .map(|item| item.id)
            .collect();
        if !items.is_empty() {
            self.replace_selection(items);
        }
        Ok(())
    }
    /// The selection changed: end the open scrub and drop every nested scope.
    pub(in crate::editor) fn selection_changed(&mut self) {
        self.finish_scrub(false);
        self.player.key_color_pick = None;
        self.player.matte_brush = None;
        self.player.brush_trail = None;
        self.selection_scopes = SelectionScopes::default();
        self.properties.clear_drafts();
    }
    /// A timeline transition replaces the item selection, so the inspector edits it.
    pub(in crate::editor) fn select_transition(&mut self, transition: project::TransitionId) {
        if !self.transition_exists(transition) {
            return;
        }
        self.selection_changed();
        if self.write_selection(BTreeSet::new()) {
            self.selection_scopes.transition = Some(transition);
        }
    }
    fn transition_exists(&self, transition: project::TransitionId) -> bool {
        self.session
            .project()
            .data()
            .scene()
            .timeline
            .transitions
            .iter()
            .any(|tr| tr.id == transition)
    }
    /// History and external commands can remove the selected transition.
    pub(in crate::editor) fn forget_missing_transition(&mut self) {
        if self
            .selection_scopes
            .transition
            .is_some_and(|transition| !self.transition_exists(transition))
        {
            self.selection_scopes.transition = None;
            self.properties.clear_drafts();
        }
    }
    /// Delete removes the selected transition as one undo step.
    pub(in crate::editor) fn delete_selected_transition(&mut self) {
        let Some(transition) = self.selection_scopes.transition else {
            return;
        };
        let result = self.apply_result(engine::Command::RemoveTransition { transition });
        match result {
            Ok(_) => {
                self.selection_scopes.transition = None;
                self.properties.clear_drafts();
            }
            Err(error) => self.fail(error.to_string()),
        }
    }
    /// The session prunes vanished items from the selection itself; the nested
    /// scopes follow it here.
    pub(in crate::editor) fn prune_selection(&mut self) {
        self.forget_missing_transition();
        let session = &self.session;
        let selected = session.selection();
        self.selection_scopes.mask_points.retain(|target| {
            selected.contains(&target.item)
                && session
                    .project()
                    .data()
                    .scene()
                    .timeline
                    .iter()
                    .flat_map(|track| track.items.iter())
                    .find(|item| item.id == target.item)
                    .is_some_and(|item| {
                        item.masks.iter().any(|mask| {
                            mask.id == target.mask
                                && matches!(
                                    &mask.shape,
                                    project::MaskShape::Path { path }
                                        if target.point < path.vertices.len()
                                )
                        })
                    })
        });
        self.selection_scopes
            .keyframes
            .retain(|target| selected.contains(&target.item) && keyframe_exists(session, target));
    }
    /// Open `scene`: the scene row's switch, shared by the scene sheet and the Media
    /// Timeline page. Gestures end and the item selection clears first.
    pub(in crate::editor) fn switch_scene(&mut self, scene: project::SceneId) -> Result<()> {
        self.cancel_interaction();
        self.session.apply(engine::Command::SwitchScene { scene })?;
        self.session.set_selection(Default::default())?;
        self.selection_scopes = SelectionScopes::default();
        Ok(())
    }
    pub(in crate::editor) fn clear_scene_selection(&mut self) {
        self.timeline.scenes.clear_selection();
        self.selection_scopes = SelectionScopes::default();
        self.write_selection(BTreeSet::new());
    }
    pub(in crate::editor) fn selection_by_end(&self, descending: bool) -> Vec<ItemId> {
        let selected = self.session.selection();
        let mut items: Vec<(Tick, ItemId)> = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter())
            .filter(|item| selected.contains(&item.id))
            .map(|item| (item.start, item.id))
            .collect();
        items.sort_by(|a, b| {
            if descending {
                b.0.cmp(&a.0)
            } else {
                a.0.cmp(&b.0)
            }
        });
        items.into_iter().map(|(_, id)| id).collect()
    }
}

fn keyframe_exists(
    session: &engine::Session,
    target: &crate::editor::selection::KeyframeSelection,
) -> bool {
    let Some(param) = session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|item| item.id == target.item)
        .and_then(|item| item.animations.get(&target.path))
    else {
        return false;
    };
    let channel = match (param, target.component) {
        (project::Param::Value(channel), None) => Some(channel),
        (project::Param::Color(channels), Some(component)) => {
            let index = match component {
                project::ColorComponent::R => 0,
                project::ColorComponent::G => 1,
                project::ColorComponent::B => 2,
                project::ColorComponent::A => 3,
            };
            channels.get(index)
        }
        _ => None,
    };
    channel.is_some_and(|channel| {
        channel
            .keys()
            .binary_search_by_key(&target.time, |key| key.time)
            .is_ok()
    })
}
