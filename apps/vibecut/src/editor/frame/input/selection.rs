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
        self.key_color_pick = None;
        self.selection_scopes = SelectionScopes::default();
        self.properties.clear_drafts();
        self.desk().select();
    }
    pub(in crate::editor) fn prune_selection(&mut self) {
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
        self.selection_scopes
            .mask_points
            .retain(|target| self.selection.contains(&target.item));
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
