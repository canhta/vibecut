//! Applies effect stack changes.

use crate::editor::properties::{
    Arc, BTreeSet, Editor, ElementKind, ItemId, Result, Text, effect_params_from_definition,
    error_detail,
};

impl Editor {
    pub(in crate::editor) fn effect_refusal(&self, id: ItemId) -> Option<Text> {
        let data = self.session.project().data();
        let track = data
            .scene()
            .timeline
            .iter()
            .find(|track| track.items.iter().any(|item| item.id == id))?;
        let (kind, _) = self.element_kind(id)?;
        if track.locked {
            return Some(Text::EffectLocked);
        }
        if kind == ElementKind::Audio {
            return Some(Text::EffectAudioUnsupported);
        }
        if !matches!(
            kind,
            ElementKind::Video | ElementKind::Image | ElementKind::Sticker | ElementKind::Graphic
        ) {
            return Some(Text::EffectKindUnsupported);
        }
        None
    }
    pub(in crate::editor) fn add_effect_to_item(&mut self, item: ItemId, kind: &str) {
        if let Some(reason) = self.effect_refusal(item) {
            let text = self.tr(reason).to_owned();
            self.set_toast(&text, None);
            return;
        }
        let Some(definition) = project::effect_definitions()
            .iter()
            .find(|definition| definition.kind == kind)
        else {
            let text = self.tr(Text::EffectKindUnsupported).to_owned();
            self.set_toast(&text, None);
            return;
        };
        let Some(params) = effect_params_from_definition(definition) else {
            let text = self.tr(Text::EffectKindUnsupported).to_owned();
            self.set_toast(&text, None);
            return;
        };
        let Some(effect_id) = self.next_effect_id(item, kind) else {
            let text = self.tr(Text::EffectLimit).to_owned();
            self.set_toast(&text, None);
            return;
        };
        let effect = project::EffectInstance {
            id: effect_id,
            enabled: true,
            params,
        };
        let before = self.session.project().clone();
        match self.session.execute(
            "effects.add",
            serde_json::json!({"item": item, "effect": effect}),
        ) {
            Ok(_) => {
                self.select_single(item);
                let text = self.tr(Text::EffectAdded).to_owned();
                let name = self.effect_name(definition).to_owned();
                self.set_toast(&text, Some(name));
            }
            Err(error) => {
                let text = if matches!(
                    &error,
                    engine::EditError::Project(project::Error::TooLarge(_))
                ) {
                    self.tr(Text::EffectLimit).to_owned()
                } else {
                    error_detail(&error)
                };
                self.set_toast(&text, None);
            }
        }
        if !Arc::ptr_eq(&before, self.session.project()) {
            self.sync_preview();
        }
    }
    pub(in crate::editor) fn next_effect_id(&self, item: ItemId, kind: &str) -> Option<String> {
        let effects = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)?
            .effects
            .iter()
            .map(|effect| effect.id.as_str())
            .collect::<BTreeSet<_>>();
        (1..=256).find_map(|index| {
            let candidate = if index == 1 {
                kind.to_owned()
            } else {
                format!("{kind}{index}")
            };
            (candidate.len() <= 64 && !effects.contains(candidate.as_str())).then_some(candidate)
        })
    }
    pub(in crate::editor) fn remove_effect(&mut self, item: ItemId, effect: &str) {
        let before = self.session.project().clone();
        match self.session.execute(
            "effects.remove",
            serde_json::json!({"item": item, "effect": effect}),
        ) {
            Ok(_) => {}
            Err(error) => self.error = Some(error.to_string()),
        }
        if !Arc::ptr_eq(&before, self.session.project()) {
            self.sync_preview();
        }
    }
    pub(in crate::editor) fn toggle_effect(&mut self, item: ItemId, effect: &str, enabled: bool) {
        let before = self.session.project().clone();
        match self.session.execute(
            "effects.toggle",
            serde_json::json!({"item": item, "effect": effect, "enabled": enabled}),
        ) {
            Ok(_) => {}
            Err(error) => self.error = Some(error.to_string()),
        }
        if !Arc::ptr_eq(&before, self.session.project()) {
            self.sync_preview();
        }
    }
    pub(in crate::editor) fn reorder_effect(
        &mut self,
        item: ItemId,
        effect: &str,
        target_effect: &str,
    ) {
        let Some(current) = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)
            .map(|candidate| {
                candidate
                    .effects
                    .iter()
                    .map(|effect| effect.id.clone())
                    .collect::<Vec<_>>()
            })
        else {
            return;
        };
        let Some(from) = current.iter().position(|id| id == effect) else {
            let text = self.tr(Text::EffectMissing).to_owned();
            self.set_toast(&text, None);
            return;
        };
        let Some(target) = current.iter().position(|id| id == target_effect) else {
            return;
        };
        let mut order = current.clone();
        let moved = order.remove(from);
        order.insert(target, moved);
        if order == current {
            return;
        }
        let before_project = self.session.project().clone();
        match self.session.execute(
            "effects.reorder",
            serde_json::json!({"item": item, "order": order}),
        ) {
            Ok(_) => {}
            Err(error) => self.error = Some(error.to_string()),
        }
        if !Arc::ptr_eq(&before_project, self.session.project()) {
            self.sync_preview();
        }
    }
    pub(in crate::editor) fn set_effect_param(
        &mut self,
        item: ItemId,
        effect: &str,
        key: &str,
        value: f64,
    ) -> Result<()> {
        self.session.set_selection(self.selection.clone())?;
        self.session.execute(
            "effects.update_param",
            serde_json::json!({"item": item, "effect": effect, "key": key, "value": value}),
        )?;
        self.sync_preview();
        Ok(())
    }
    pub(in crate::editor) fn effect_param_value(
        &self,
        item: ItemId,
        effect: &str,
        key: &str,
    ) -> Option<f64> {
        let effect = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)?
            .effects
            .iter()
            .find(|candidate| candidate.id == effect)?;
        match (&effect.params, key) {
            (project::EffectParams::Blur { intensity }, "intensity") => Some(*intensity),
            _ => None,
        }
    }
}
