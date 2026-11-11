//! Applies effects commands to a candidate project.
use super::{EditError, Item, ItemId, ItemParams, ProjectData, Result, item};

pub(super) fn effect_item(data: &mut ProjectData, id: ItemId) -> Result<&mut Item> {
    let target = item(data, id)?;
    if matches!(target.content, ItemParams::Audio(_)) {
        return Err(EditError::Unsupported("visual effect on audio item"));
    }
    Ok(target)
}
pub(super) fn execute_add_effect(
    mut data: ProjectData,
    id: ItemId,
    effect: project::EffectInstance,
) -> Result<ProjectData> {
    let item = effect_item(&mut data, id)?;
    item.effects.push(effect);

    Ok(data)
}
pub(super) fn execute_remove_effect(
    mut data: ProjectData,
    id: ItemId,
    effect: String,
) -> Result<ProjectData> {
    let item = effect_item(&mut data, id)?;
    let index = item
        .effects
        .iter()
        .position(|e| e.id == effect)
        .ok_or(EditError::Invalid("missing effect"))?;
    item.effects.remove(index);
    let prefix = format!("effects.{effect}.params.");
    let paths: Vec<_> = item
        .animations
        .iter()
        .filter(|(p, _)| p.as_str().starts_with(&prefix))
        .map(|(p, _)| p.clone())
        .collect();
    for path in paths {
        item.animations.remove(&path);
    }

    Ok(data)
}
pub(super) fn execute_update_effect_param(
    mut data: ProjectData,
    id: ItemId,
    effect: String,
    key: String,
    value: project::EffectParamUpdate,
) -> Result<ProjectData> {
    let target = effect_item(&mut data, id)?;
    let effect = target
        .effects
        .iter_mut()
        .find(|e| e.id == effect)
        .ok_or(EditError::Invalid("missing effect"))?;
    effect.set_param(&key, value)?;

    Ok(data)
}
pub(super) fn execute_toggle_effect(
    mut data: ProjectData,
    id: ItemId,
    effect: String,
    enabled: bool,
) -> Result<ProjectData> {
    let target = effect_item(&mut data, id)?;
    target
        .effects
        .iter_mut()
        .find(|e| e.id == effect)
        .ok_or(EditError::Invalid("missing effect"))?
        .enabled = enabled;

    Ok(data)
}
pub(super) fn execute_reorder_effects(
    mut data: ProjectData,
    id: ItemId,
    order: Vec<String>,
) -> Result<ProjectData> {
    let target = effect_item(&mut data, id)?;
    if order.len() != target.effects.len() {
        return Err(EditError::Invalid("effect order length"));
    }
    let mut reordered = Vec::with_capacity(order.len());
    for id in order {
        let index = target
            .effects
            .iter()
            .position(|e| e.id == id)
            .ok_or(EditError::Invalid("effect order identity"))?;
        reordered.push(target.effects.remove(index));
    }
    target.effects = reordered;

    Ok(data)
}

/// Register and apply one completed import as one ordinary atomic batch.
impl crate::Session {
    pub fn apply_lut(
        &mut self,
        item: ItemId,
        record: project::LutRecord,
        strength: f64,
    ) -> Result<crate::Outcome> {
        let target = self
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|t| &t.items)
            .find(|i| i.id == item)
            .ok_or(EditError::NoItem(item))?;
        let current = target.effects.iter().find(|e| e.kind == "color_grade");
        let mut commands = vec![crate::Command::RegisterLut {
            record: record.clone(),
        }];
        let effect = if let Some(effect) = current {
            effect.id.clone()
        } else {
            let id = (1..=256)
                .map(|i| format!("color_grade{i}"))
                .find(|id| !target.effects.iter().any(|e| &e.id == id))
                .ok_or(EditError::Limit)?;
            commands.push(crate::Command::AddEffect {
                item,
                effect: project::effect_definition("color_grade")?.instance(&id),
            });
            id
        };
        commands.push(crate::Command::UpdateEffectParam {
            item,
            effect: effect.clone(),
            key: "lut".into(),
            value: project::EffectParamUpdate::LutRef(project::LutReference {
                lut: Some(record.id),
            }),
        });
        commands.push(crate::Command::UpdateEffectParam {
            item,
            effect,
            key: "lut_strength".into(),
            value: project::EffectParamUpdate::Scalar(strength),
        });
        self.apply(crate::Command::Batch { commands })
    }
}
