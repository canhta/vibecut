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
        if self.scrub.is_some() {
            self.finish_scrub(false);
        }
        self.selection_scopes = SelectionScopes::default();
        self.property_drafts.clear();
        if let Err(e) = self.session.set_selection(self.selection.clone()) {
            self.error = Some(e.to_string());
        }
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
        self.selection_scopes
            .keyframes
            .retain(|target| self.selection.contains(&target.item));
    }
    pub(in crate::editor) fn clear_scene_selection(&mut self) {
        self.scene_selected.clear();
        self.scene_select = false;
        self.scene_confirm = false;
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
