//! Maintains editor selection scope.

use crate::editor::{BTreeSet, Editor, ItemId, SelectionScopes, Tick, egui};

impl Editor {
    pub(in crate::editor) fn select_single(&mut self, item: ItemId) {
        self.selection.clear();
        self.selection.insert(item);
        self.selection_changed();
    }
    pub(in crate::editor) fn select_item(&mut self, item: ItemId, modifiers: egui::Modifiers) {
        crate::selection::click(
            &mut self.selection,
            &mut None,
            item,
            modifiers.command || modifiers.ctrl || modifiers.shift,
            None,
        );
        self.selection_changed();
    }
    pub(in crate::editor) fn select_all(&mut self) {
        self.selection = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter().map(|item| item.id))
            .collect();
        self.selection_changed();
    }
    pub(in crate::editor) fn selection_changed(&mut self) {
        self.finish_scrub(false);
        self.player.key_color_pick = None;
        self.selection_scopes = SelectionScopes::default();
        self.properties.clear_drafts();
        self.desk().select();
    }
    /// A timeline transition replaces the item selection, so the inspector edits it.
    pub(in crate::editor) fn select_transition(&mut self, transition: project::TransitionId) {
        if !self.transition_exists(transition) {
            return;
        }
        self.finish_scrub(false);
        self.player.key_color_pick = None;
        self.selection.clear();
        self.selection_scopes = SelectionScopes::default();
        self.selection_scopes.transition = Some(transition);
        self.properties.clear_drafts();
        self.desk().select();
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
        let result = self
            .desk()
            .apply_result(engine::Command::RemoveTransition { transition }, false);
        match result {
            Ok(_) => {
                self.selection_scopes.transition = None;
                self.properties.clear_drafts();
            }
            Err(error) => self.desk().fail(error.to_string()),
        }
    }
    pub(in crate::editor) fn prune_selection(&mut self) {
        self.forget_missing_transition();
        let existing: BTreeSet<ItemId> = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter().map(|item| item.id))
            .collect();
        self.selection.retain(|id| existing.contains(id));
        let session = &self.session;
        self.selection_scopes.mask_points.retain(|target| {
            self.selection.contains(&target.item)
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
        let selected = self.selection.clone();
        let session = &self.session;
        self.selection_scopes
            .keyframes
            .retain(|target| selected.contains(&target.item) && keyframe_exists(session, target));
    }
    pub(in crate::editor) fn clear_scene_selection(&mut self) {
        self.timeline.scenes.clear_selection();
        self.selection.clear();
        self.selection_scopes = SelectionScopes::default();
    }
    pub(in crate::editor) fn selection_by_end(&self, descending: bool) -> Vec<ItemId> {
        let mut items: Vec<(Tick, ItemId)> = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter())
            .filter(|item| self.selection.contains(&item.id))
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
    target: &crate::selection::KeyframeSelection,
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
