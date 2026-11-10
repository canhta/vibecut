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
                crate::Error::Engine(engine::EditError::Project(project::Error::TooLarge(_)))
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
    desk.apply_result(
        engine::Command::UpdateEffectParam {
            item,
            effect: effect.to_owned(),
            key: "key_color".into(),
            value: packed,
        },
        false,
    )?;
    Ok(())
}

pub(super) fn remove_effect(desk: &mut Desk<'_>, item: ItemId, effect: &str) {
    let _ = desk.apply_result(
        engine::Command::RemoveEffect {
            item,
            effect: effect.to_owned(),
        },
        false,
    );
}

pub(super) fn toggle_effect(desk: &mut Desk<'_>, item: ItemId, effect: &str, enabled: bool) {
    let _ = desk.apply_result(
        engine::Command::ToggleEffect {
            item,
            effect: effect.to_owned(),
            enabled,
        },
        false,
    );
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
    let _ = desk.apply_result(
        engine::Command::ReorderEffects {
            item,
            order: order.clone(),
        },
        false,
    );
}

pub(super) fn set_effect_param(
    desk: &mut Desk<'_>,
    item: ItemId,
    effect: &str,
    key: &str,
    value: f64,
) -> Result<()> {
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
        desk.apply_result(
            engine::Command::UpdateEffectParam {
                item,
                effect: effect.to_owned(),
                key: key.to_owned(),
                value,
            },
            false,
        )?;
    } else {
        // First edit on a panel that shows a registered effect at neutral:
        // add it carrying the edited value, inside the open gesture.
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
        instance.set_scalar(key, value)?;
        desk.apply_result(
            engine::Command::AddEffect {
                item,
                effect: instance,
            },
            false,
        )?;
    }
    desk.sync();
    Ok(())
}
