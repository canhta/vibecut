//! Applies effect stack changes.

use crate::editor::properties::{
    Arc, BTreeSet, Editor, ElementKind, Error, ItemId, Result, Text, effect_params_from_definition,
    error_detail, read,
};

impl Editor {
    /// The Remove BG toggle: presence plus enabled, so the panel never shows
    /// keyed controls while the effect bypasses the frame.
    pub(in crate::editor::properties) fn toggle_chroma_key(&mut self, item: ItemId, enabled: bool) {
        let existing = read::chroma_key(self.session.project(), item)
            .map(|effect| (effect.id.clone(), effect.enabled));
        match (existing, enabled) {
            (None, false) => {}
            (None, true) => self.add_effect_to_item(item, "chroma_key"),
            (Some((id, on)), want) if on != want => self.toggle_effect(item, &id, want),
            (Some(_), _) => {}
        }
    }

    /// Restore the catalog's default key colour in one undo step.
    pub(in crate::editor::properties) fn reset_key_color(
        &mut self,
        item: ItemId,
        effect: &str,
    ) -> Result<()> {
        let default = project::chroma_key_color(project::CHROMA_KEY_COLOR_DEFAULT)?;
        let current = read::chroma_key_color(self.session.project(), item, effect);
        if current == Some(default) {
            return Ok(());
        }
        self.set_key_color(item, effect, default)
    }

    /// Write one key colour as the packed scalar the effect command admits.
    pub(in crate::editor) fn set_key_color(
        &mut self,
        item: ItemId,
        effect: &str,
        color: project::Color,
    ) -> Result<()> {
        let [r, g, b, _] = color.0;
        let packed = (u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)) as f64;
        self.desk().edit_result(
            "effects.update_param",
            serde_json::json!({"item": item, "effect": effect, "key": "key_color", "value": packed}),
        )?;
        Ok(())
    }

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
            self.desk().sync();
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
        self.desk().edit(
            "effects.remove",
            serde_json::json!({"item": item, "effect": effect}),
        );
    }
    pub(in crate::editor) fn toggle_effect(&mut self, item: ItemId, effect: &str, enabled: bool) {
        self.desk().edit(
            "effects.toggle",
            serde_json::json!({"item": item, "effect": effect, "enabled": enabled}),
        );
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
        self.desk().edit(
            "effects.reorder",
            serde_json::json!({"item": item, "order": order}),
        );
    }
    pub(in crate::editor) fn set_effect_param(
        &mut self,
        item: ItemId,
        effect: &str,
        key: &str,
        value: f64,
    ) -> Result<()> {
        let present = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)
            .is_some_and(|candidate| candidate.effects.iter().any(|e| e.id == effect));
        if present {
            self.desk().run(
                "effects.update_param",
                serde_json::json!({"item": item, "effect": effect, "key": key, "value": value}),
                false,
            )?;
        } else {
            // First edit on a panel that shows a registered effect at neutral:
            // add it carrying the edited value, inside the open gesture.
            if let Some(reason) = self.effect_refusal(item) {
                return Err(Error::Invalid(match reason {
                    Text::EffectLocked => "track is locked",
                    _ => "effect unsupported on this item",
                }));
            }
            let mut params = project::effect_definitions()
                .iter()
                .find(|definition| definition.kind == effect)
                .and_then(effect_params_from_definition)
                .ok_or(Error::Invalid("unknown effect"))?;
            params.set_scalar(key, value)?;
            let instance = project::EffectInstance {
                id: effect.to_owned(),
                enabled: true,
                params,
            };
            self.desk().run(
                "effects.add",
                serde_json::json!({"item": item, "effect": instance}),
                false,
            )?;
        }
        self.desk().sync();
        Ok(())
    }
}
