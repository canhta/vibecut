//! Applies effect stack changes.
use super::super::{ElementKind, State, element_kind, read};
use crate::editor::{BTreeSet, Desk, Error, ItemId, Result, Text, error_detail};

/// The Remove BG switch: on with no effect stages the eyedropper, so the
/// first pick adds the effect; off removes an existing effect. Enabling a
/// bypassed effect is a toggle, not a second instance.
impl State {
    pub(super) fn toggle_chroma_key(&mut self, desk: &mut Desk<'_>, item: ItemId, enabled: bool) {
        let existing = read::chroma_key(desk.view().session.project(), item)
            .map(|effect| (effect.id.clone(), effect.enabled));
        match (enabled, existing) {
            (true, Some((id, false))) => {
                toggle_effect(desk, item, &id, true);
                self.chroma_staged = None;
            }
            (true, Some((_, true))) => self.chroma_staged = None,
            (true, None) => self.chroma_staged = Some(item),
            (false, Some((id, _))) => {
                remove_effect(desk, item, &id);
                self.chroma_staged = None;
            }
            (false, None) => self.chroma_staged = None,
        }
    }

    /// One eyedropper pick: add the missing effect carrying the colour, or
    /// update an existing effect. The staged switch clears only on success.
    pub(in crate::editor) fn pick_key_color(
        &mut self,
        desk: &mut Desk<'_>,
        item: ItemId,
        effect: Option<&str>,
        color: project::Color,
    ) -> Result<()> {
        let existing = effect.map(str::to_owned).or_else(|| {
            read::chroma_key(desk.view().session.project(), item).map(|effect| effect.id.clone())
        });
        if let Some(effect) = existing {
            set_key_color(desk, item, &effect, color)?;
        } else {
            let [r, g, b, _] = color.0;
            let packed = (u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)) as f64;
            set_effect_param(desk, item, "chroma_key", "key_color", packed)?;
        }
        self.chroma_staged = None;
        Ok(())
    }

    /// Restore the catalog's default key colour in one undo step.
    pub(super) fn reset_key_color(
        &mut self,
        desk: &mut Desk<'_>,
        item: ItemId,
        effect: &str,
    ) -> Result<()> {
        let default = project::CHROMA_KEY_COLOR_DEFAULT;
        let current = read::chroma_key_color(desk.view().session.project(), item, effect);
        if current == Some(default) {
            return Ok(());
        }
        set_key_color(desk, item, effect, default)
    }
}

/// Why an item refuses an effect, from project data.
pub(in crate::editor) fn effect_refusal(project: &project::Project, id: ItemId) -> Option<Text> {
    let data = project.data();
    let track = data
        .scene()
        .timeline
        .iter()
        .find(|track| track.items.iter().any(|item| item.id == id))?;
    let (kind, _) = element_kind(project, id)?;
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

/// Attach one effect definition to a clip's stack as one undo step.
pub(in crate::editor) fn add_effect_to_item(desk: &mut Desk<'_>, item: ItemId, kind: &str) {
    if let Some(reason) = effect_refusal(desk.view().session.project(), item) {
        let text = crate::i18n::text(desk.view().preferences.locale, reason).to_owned();
        desk.toast(&text, None);
        return;
    }
    let Some(definition) = project::effect_definitions()
        .iter()
        .find(|definition| definition.id == kind)
    else {
        let text = crate::i18n::text(desk.view().preferences.locale, Text::EffectKindUnsupported)
            .to_owned();
        desk.toast(&text, None);
        return;
    };
    let Some(effect_id) = next_effect_id(desk.view().session.project(), item, kind) else {
        let text = crate::i18n::text(desk.view().preferences.locale, Text::EffectLimit).to_owned();
        desk.toast(&text, None);
        return;
    };
    let effect = project::EffectInstance {
        id: effect_id,
        kind: definition.id.to_owned(),
        enabled: true,
        params: definition.defaults(),
    };
    let before = desk.view().session.project().clone();
    match desk
        .with_session(|session| Ok(session.apply(engine::Command::AddEffect { item, effect })?))
    {
        Ok(_) => {
            desk.select_single(item);
            let text =
                crate::i18n::text(desk.view().preferences.locale, Text::EffectAdded).to_owned();
            let name = crate::i18n::effect_label(definition.id, None)
                .map_or(definition.name, |key| {
                    crate::i18n::text(desk.view().preferences.locale, key)
                })
                .to_owned();
            desk.toast(&text, Some(name));
        }
        Err(error) => {
            let text = if matches!(
                &error,
                crate::Error::Engine(engine::control::Error::Edit(engine::EditError::Project(
                    project::Error::TooLarge(_)
                )))
            ) {
                crate::i18n::text(desk.view().preferences.locale, Text::EffectLimit).to_owned()
            } else {
                error_detail(&error)
            };
            desk.toast(&text, None);
        }
    }
    if !std::sync::Arc::ptr_eq(&before, desk.view().session.project()) {
        desk.sync();
    }
}

/// The next free instance id for `kind` on one clip.
pub(super) fn next_effect_id(
    project: &project::Project,
    item: ItemId,
    kind: &str,
) -> Option<String> {
    let effects = project
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

/// Write one key colour as the packed scalar the effect command admits.
fn set_key_color(
    desk: &mut Desk<'_>,
    item: ItemId,
    effect: &str,
    color: project::Color,
) -> Result<()> {
    let [r, g, b, _] = color.0;
    let packed = (u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)) as f64;
    desk.apply_result(engine::Command::UpdateEffectParam {
        item,
        effect: effect.to_owned(),
        key: "key_color".into(),
        value: project::EffectParamUpdate::Scalar(packed),
    })?;
    Ok(())
}

pub(super) fn remove_effect(desk: &mut Desk<'_>, item: ItemId, effect: &str) {
    let _ = desk.apply_result(engine::Command::RemoveEffect {
        item,
        effect: effect.to_owned(),
    });
}

pub(super) fn toggle_effect(desk: &mut Desk<'_>, item: ItemId, effect: &str, enabled: bool) {
    let _ = desk.apply_result(engine::Command::ToggleEffect {
        item,
        effect: effect.to_owned(),
        enabled,
    });
}

pub(super) fn reorder_effect(desk: &mut Desk<'_>, item: ItemId, effect: &str, target_effect: &str) {
    let Some(current) = desk
        .view()
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
        let text =
            crate::i18n::text(desk.view().preferences.locale, Text::EffectMissing).to_owned();
        desk.toast(&text, None);
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
    let _ = desk.apply_result(engine::Command::ReorderEffects {
        item,
        order: order.clone(),
    });
}

pub(super) fn set_effect_param(
    desk: &mut Desk<'_>,
    item: ItemId,
    effect: &str,
    key: &str,
    value: f64,
) -> Result<()> {
    set_effect_values(
        desk,
        item,
        effect,
        &[(key.to_owned(), project::EffectParamUpdate::Scalar(value))],
    )
}

/// Replace one or more typed values; a multi-value reset is one atomic batch.
pub(super) fn set_effect_values(
    desk: &mut Desk<'_>,
    item: ItemId,
    effect: &str,
    values: &[(String, project::EffectParamUpdate)],
) -> Result<()> {
    if values.is_empty() {
        return Ok(());
    }
    let present = desk
        .view()
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
        let mut commands: Vec<engine::Command> = values
            .iter()
            .map(|(key, value)| engine::Command::UpdateEffectParam {
                item,
                effect: effect.to_owned(),
                key: key.clone(),
                value: *value,
            })
            .collect();
        let command = if commands.len() == 1 {
            commands
                .pop()
                .ok_or(Error::Invalid("effect parameter values"))?
        } else {
            engine::Command::Batch { commands }
        };
        desk.apply_result(command)?;
    } else {
        // First edit on a panel that shows a registered effect at neutral:
        // add it carrying the edited values, inside the open gesture.
        if let Some(reason) = effect_refusal(desk.view().session.project(), item) {
            return Err(Error::Invalid(match reason {
                Text::EffectLocked => "track is locked",
                _ => "effect unsupported on this item",
            }));
        }
        let mut instance = project::effect_definitions()
            .iter()
            .find(|definition| definition.id == effect)
            .map(|definition| definition.instance(effect))
            .ok_or(Error::Invalid("unknown effect"))?;
        for (key, value) in values {
            instance.set_param(key, *value)?;
        }
        desk.apply_result(engine::Command::AddEffect {
            item,
            effect: instance,
        })?;
    }
    desk.sync();
    Ok(())
}

/// Copy the selected item's complete grade onto every unlocked video and image
/// item as one `batch` command, so the whole copy is one undo step. Items that
/// already carry a grade receive its values; the rest gain an instance.
pub(super) fn apply_grade_to_all(desk: &mut Desk<'_>, source: ItemId) -> Result<()> {
    let project = desk.view().session.project();
    let Some(params) = read::item(project, source)
        .and_then(|item| {
            item.effects
                .iter()
                .find(|effect| effect.kind == "color_grade")
        })
        .map(|effect| effect.params.clone())
    else {
        return Ok(());
    };
    let mut commands = Vec::new();
    for track in project
        .data()
        .scene()
        .timeline
        .iter()
        .filter(|track| !track.locked)
    {
        for item in &track.items {
            if !matches!(
                element_kind(project, item.id),
                Some((ElementKind::Video | ElementKind::Image, _))
            ) {
                continue;
            }
            if let Some(existing) = item
                .effects
                .iter()
                .find(|effect| effect.kind == "color_grade")
            {
                for (key, value) in &params {
                    if let Some(value) = value.update() {
                        commands.push(engine::Command::UpdateEffectParam {
                            item: item.id,
                            effect: existing.id.clone(),
                            key: key.clone(),
                            value,
                        });
                    }
                }
            } else if let Some(id) = next_effect_id(project, item.id, "color_grade") {
                let mut instance = project::effect_definition("color_grade")?.instance(id);
                instance.params = params.clone();
                commands.push(engine::Command::AddEffect {
                    item: item.id,
                    effect: instance,
                });
            }
        }
    }
    if commands.is_empty() {
        return Ok(());
    }
    desk.apply_result(engine::Command::Batch { commands })?;
    desk.sync();
    Ok(())
}

/// Store the selected item's grade as the next free app-owned preset name.
pub(super) fn save_grade_preset(desk: &mut Desk<'_>, source: ItemId) -> Result<()> {
    let project = desk.view().session.project();
    let Some(params) = read::item(project, source)
        .and_then(|item| {
            item.effects
                .iter()
                .find(|effect| effect.kind == "color_grade")
        })
        .map(|effect| effect.params.clone())
    else {
        return Ok(());
    };
    let mut presets = desk.view().preferences.adjustment_presets.clone();
    let taken: BTreeSet<&str> = presets.iter().map(|preset| preset.name.as_str()).collect();
    let Some(number) = (1..=crate::preferences::MAX_ADJUSTMENT_PRESETS).find(|number| {
        let name = format!("Preset {number}");
        !taken.contains(name.as_str())
    }) else {
        return Ok(());
    };
    presets.push(crate::preferences::AdjustmentPreset {
        name: format!("Preset {number}"),
        params,
    });
    desk.store_preference("adjustment_presets", serde_json::to_value(presets)?)
}
