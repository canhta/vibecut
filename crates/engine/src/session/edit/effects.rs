//! Applies effects commands to a candidate project.
use super::{
    CreatedId, EditError, Item, ItemId, ItemParams, NewItem, Placement, ProjectData, Result,
    SourceRange, Tick, item,
};

/// Insert one filter clip: an effect item on an effect track whose stack is the look's colour
/// grade and whose opacity is the Intensity (0..=100 percent; 100 stores no channel).
#[allow(
    clippy::too_many_arguments,
    reason = "the edit dispatcher passes the command fields and the id allocators straight through"
)]
pub(super) fn execute_insert_filter(
    data: ProjectData,
    placement: Placement,
    filter: &str,
    start: Tick,
    duration: Tick,
    intensity: f64,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
) -> Result<ProjectData> {
    let definition = project::filter_definition(filter).ok_or(EditError::Invalid("filter"))?;
    if !intensity.is_finite() || !(0.0..=100.0).contains(&intensity) {
        return Err(EditError::Invalid("filter intensity"));
    }
    let mut animations = project::Animations::default();
    if intensity < 100.0 {
        animations.set(
            project::AnimationPath::from(project::BuiltinPath::Opacity),
            project::Param::new(project::ParamValue::Scalar(intensity / 100.0))?,
        )?;
    }
    let new = NewItem {
        animations,
        source_asset: None,
        source_range: SourceRange::new(Tick::ZERO, duration).map_err(EditError::from)?,
        start,
        retime: Default::default(),
        content: ItemParams::Effect(project::EffectParams::default()),
    };
    let mut data = super::clips::execute_insert_item(data, placement, new, created, next)?;
    let id = created
        .iter()
        .rev()
        .find_map(|created| match created {
            CreatedId::Item(id) => Some(*id),
            _ => None,
        })
        .ok_or(EditError::Invalid("filter insert"))?;
    effect_item(&mut data, id)?
        .effects
        .push(definition.instance(project::FILTER_INSTANCE_ID)?);
    Ok(data)
}

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
                .ok_or(EditError::Control(
                    crate::session::wire::ControlError::Limit,
                ))?;
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
