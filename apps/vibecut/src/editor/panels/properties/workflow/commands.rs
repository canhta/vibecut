//! Adapts property edits to engine patches.
use super::super::read;
use crate::editor::{Desk, Error, ItemId, ItemParamPatch, Result};

/// One animation slot of a clip: the slot, its look (`None` removes) and its duration.
pub(super) type SlotChange = (
    engine::AnimationSlot,
    Option<engine::AnimationPreset>,
    Option<time::Tick>,
);

pub(super) fn set_clip_animation(
    state: &mut super::State,
    desk: &mut Desk<'_>,
    item: ItemId,
    (slot, preset, duration): SlotChange,
    snapshot: std::sync::Arc<project::Project>,
) -> Result<()> {
    text_target(state, desk, item, &snapshot, "clip animation")?;
    desk.apply_result(
        engine::Command::SetClipAnimation {
            item,
            slot,
            preset,
            duration,
        },
        false,
    )?;
    Ok(())
}

pub(in crate::editor) fn apply_text_preset(
    state: &mut super::State,
    desk: &mut Desk<'_>,
    item: ItemId,
    preset: project::TextPresetId,
    snapshot: std::sync::Arc<project::Project>,
) -> Result<()> {
    text_target(state, desk, item, &snapshot, "text preset")?;
    desk.apply_result(
        engine::Command::ApplyTextPreset {
            items: vec![item],
            preset,
        },
        false,
    )?;
    Ok(())
}

pub(in crate::editor) fn save_text_preset(
    state: &mut super::State,
    desk: &mut Desk<'_>,
    item: ItemId,
    snapshot: std::sync::Arc<project::Project>,
) -> Result<()> {
    text_target(state, desk, item, &snapshot, "text preset")?;
    let name = next_preset_name(desk.view().session.project())?;
    desk.apply_result(engine::Command::SaveTextPreset { item, name }, false)?;
    Ok(())
}

/// Guard a text workflow against a stale target, open transaction or blocked editor.
fn text_target(
    state: &mut super::State,
    desk: &mut Desk<'_>,
    item: ItemId,
    snapshot: &std::sync::Arc<project::Project>,
    what: &'static str,
) -> Result<()> {
    if !desk.editor().clip_animation_available(state)
        || state.animation_target(desk.view().session, desk.view().selection()) != Some(item)
        || !std::sync::Arc::ptr_eq(snapshot, desk.view().session.project())
    {
        return Err(Error::Invalid(match what {
            "text preset" => "text preset target changed",
            _ => "clip animation target changed",
        }));
    }
    Ok(())
}

/// First free `Style N` name in the saved bank; the engine rechecks uniqueness.
fn next_preset_name(project: &project::Project) -> Result<captions::NfcText> {
    let bank = &project.data().text_presets;
    (1..=project::MAX_TEXT_PRESETS)
        .map(|n| format!("Style {n}"))
        .find(|name| !bank.iter().any(|preset| preset.name.as_str() == name))
        .map(Into::into)
        .ok_or(Error::Invalid("text preset bank is full"))
}

/// The inspector and automation use the same registered atomic field edit.
pub(in crate::editor) fn set_shared_field(
    desk: &mut Desk<'_>,
    field: &str,
    value: &str,
) -> Result<()> {
    if field == "opacity"
        && let Some(item) = crate::editor::context::single(desk.view().selection())
    {
        return set_item_field(desk, item, field, value);
    }
    let patch = item_patch(field, value)?;
    let items = desk.view().selection().iter().copied().collect();
    desk.apply_result(engine::Command::SetItemParamMany { items, patch }, false)?;
    Ok(())
}

/// Toggle one diamond over its fields at the playhead as one undo step: keys
/// are removed when every field has one, otherwise added where missing.
pub(super) fn toggle_keyframes(
    desk: &mut Desk<'_>,
    item: ItemId,
    fields: &[(String, f64)],
    time: time::Tick,
) -> Result<()> {
    let all = {
        let view = desk.view();
        fields
            .iter()
            .all(|(field, _)| read::has_key_at(view.session.project(), item, field, time))
    };
    desk.transaction_result(|session| {
        for (field, value) in fields {
            let has = read::has_key_at(session.project(), item, field, time);
            if has != all {
                continue;
            }
            session.apply(engine::Command::ToggleKeyframe {
                item,
                path: project::AnimationPath::new(field)?,
                component: None,
                time,
                value: project::ParamValue::Scalar(*value),
            })?;
        }
        Ok(())
    })?;
    Ok(())
}

/// Upsert a key while the field already has a curve; the base stays untouched.
pub(super) fn set_keyframe_at(
    desk: &mut Desk<'_>,
    item: ItemId,
    field: &str,
    time: time::Tick,
    value: f64,
) -> Result<()> {
    desk.apply_result(
        engine::Command::AddKeyframe {
            item,
            path: project::AnimationPath::new(field)?,
            component: None,
            key: project::Keyframe::new(time, project::ParamValue::Scalar(value)),
        },
        false,
    )?;
    Ok(())
}

/// Reset every authored transform channel, reflections included, as one atomic
/// batch. A keyed reflection axis refuses the batch before any channel changes.
/// A Manual fit returns to Fill, since its recorded scale is part of the reset.
pub(super) fn reset_transform(desk: &mut Desk<'_>, item: ItemId) -> Result<()> {
    let manual = read::item(desk.view().session.project(), item).is_some_and(|item| {
        matches!(
            &item.content,
            project::ItemParams::Video(params) if params.fit == project::FitMode::Manual
        )
    });
    let mut commands = Vec::with_capacity(project::TransformField::ALL.len() + 3);
    if manual {
        commands.push(engine::Command::SetItemField {
            item,
            patch: ItemParamPatch::Fit(project::FitMode::Fill),
        });
    }
    for field in project::TransformField::ALL {
        commands.push(engine::Command::SetItemField {
            item,
            patch: item_patch(field.builtin().as_str(), &field.default_value().to_string())?,
        });
    }
    for axis in project::FlipAxis::ALL {
        commands.push(engine::Command::SetItemsFlipped {
            items: vec![item],
            axis,
            flipped: false,
        });
    }
    desk.apply_result(engine::Command::Batch { commands }, false)?;
    Ok(())
}

/// Reset the Blend section's channels (blend mode and opacity) to their engine
/// defaults as one atomic batch, one undo step.
pub(super) fn reset_blend(desk: &mut Desk<'_>, item: ItemId) -> Result<()> {
    desk.apply_result(
        engine::Command::Batch {
            commands: vec![
                engine::Command::SetItemParamMany {
                    items: vec![item],
                    patch: ItemParamPatch::BlendMode(project::BlendMode::Normal),
                },
                engine::Command::SetItemParamMany {
                    items: vec![item],
                    patch: ItemParamPatch::Opacity(1.0),
                },
            ],
        },
        false,
    )?;
    Ok(())
}

/// One element's committed field edit, through the singular registered command.
pub(in crate::editor) fn set_item_field(
    desk: &mut Desk<'_>,
    item: ItemId,
    field: &str,
    value: &str,
) -> Result<()> {
    let bounds = animated_bounds(field);
    if bounds.is_some() && read::channel_has_keys(desk.view().session.project(), item, field) {
        let (start, end) = read::range_of(desk.view().session.project(), item)
            .ok_or(Error::Invalid("opacity target"))?;
        let at = desk.view().playhead;
        if at < start || at >= end {
            return Err(Error::Invalid("opacity playhead outside item"));
        }
        let (min, max) = bounds.ok_or(Error::Invalid("opacity"))?;
        let value = value
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && (min..=max).contains(value))
            .ok_or(Error::Invalid("opacity"))?;
        return set_keyframe_at(desk, item, field, at.saturating_sub(start), value);
    }
    if field == "speed" {
        let speed = value
            .trim()
            .parse::<f64>()
            .map_err(|_| Error::Invalid("speed"))?;
        let (_, maintain_pitch, _) =
            read::retime_of(desk.view().session.project(), item).ok_or(Error::Invalid("speed"))?;
        return super::speed::set_speed(desk, item, speed, maintain_pitch);
    }
    if let Some((min, max)) = animated_bounds(field).filter(|_| field.starts_with("params.")) {
        let value = value
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && (min..=max).contains(v))
            .ok_or(Error::Invalid("graphic scalar"))?;
        desk.apply_result(
            engine::Command::SetAnimatedParam {
                item,
                path: project::AnimationPath::new(field)?,
                param: project::Param::new(project::ParamValue::Scalar(value))?,
            },
            false,
        )?;
        return Ok(());
    }
    let patch = item_patch(field, value)?;
    desk.apply_result(engine::Command::SetItemField { item, patch }, false)?;
    Ok(())
}

/// Admitted range of a field that edits through its curve while it is keyed: opacity and
/// the graphic scalars, as the engine's parameter table declares them.
fn animated_bounds(field: &str) -> Option<(f64, f64)> {
    (field == "opacity" || field.starts_with("params."))
        .then(|| read::path_bounds(field))
        .flatten()
}

/// The patch a field name and its text stand for, read by the engine's parameter table.
fn item_patch(field: &str, value: &str) -> Result<ItemParamPatch> {
    ItemParamPatch::parse(field, value).map_err(|error| {
        Error::Invalid(match error {
            engine::ParamParseError::Unknown => "shared field",
            engine::ParamParseError::Value => "field value",
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_table_field_parses_by_its_wire_name() {
        for row in engine::ITEM_PARAMS {
            assert!(
                !matches!(
                    item_patch(row.name, ""),
                    Err(Error::Invalid("shared field"))
                ),
                "{} is not parsed by its wire name",
                row.name
            );
        }
        assert!(matches!(
            item_patch("audio_gain", "1"),
            Err(Error::Invalid("shared field"))
        ));
    }
}
