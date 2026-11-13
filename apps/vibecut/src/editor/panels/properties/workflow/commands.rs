//! Adapts property edits to engine patches.
use super::super::read;
use crate::editor::{Desk, Error, ItemId, ItemParamPatch, Result};

pub(in crate::editor) fn apply_text_animation(
    state: &mut super::State,
    desk: &mut Desk<'_>,
    item: ItemId,
    preset: engine::TextAnimationPreset,
    snapshot: std::sync::Arc<project::Project>,
) -> Result<()> {
    text_target(state, desk, item, &snapshot, "text animation")?;
    desk.apply_result(engine::Command::ApplyTextAnimation { item, preset }, false)?;
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
    if !desk.editor().text_animation_available(state)
        || state.animation_target(desk.view().session, desk.view().selection()) != Some(item)
        || !std::sync::Arc::ptr_eq(snapshot, desk.view().session.project())
    {
        return Err(Error::Invalid(match what {
            "text preset" => "text preset target changed",
            _ => "text animation target changed",
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
            let path = if field == "audio_gain" {
                project::BuiltinPath::Volume.as_str()
            } else {
                field.as_str()
            };
            session.apply(engine::Command::ToggleKeyframe {
                item,
                path: project::AnimationPath::new(path)?,
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
    let path = if field == "audio_gain" {
        project::AnimationPath::from(project::BuiltinPath::Volume)
    } else {
        project::AnimationPath::new(field)?
    };
    desk.apply_result(
        engine::Command::AddKeyframe {
            item,
            path,
            component: None,
            key: project::Keyframe::new(time, project::ParamValue::Scalar(value)),
        },
        false,
    )?;
    Ok(())
}

/// Reset every authored transform channel, reflections included, as one atomic
/// batch. A keyed reflection axis refuses the batch before any channel changes.
pub(super) fn reset_transform(desk: &mut Desk<'_>, item: ItemId) -> Result<()> {
    let mut commands = Vec::with_capacity(project::TransformField::ALL.len() + 2);
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
    let bounds = if field == "opacity" {
        Some((0., 1.))
    } else {
        scalar_bounds(field)
    };
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
    if let Some((min, max)) = scalar_bounds(field) {
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

/// Admitted range of a graphic scalar parameter, in authored units.
fn scalar_bounds(field: &str) -> Option<(f64, f64)> {
    match field {
        "params.strokeWidth" => Some((0., 64.)),
        "params.cornerRadius" => Some((0., 50.)),
        "params.sides" | "params.points" => Some((3., 12.)),
        "params.depth" => Some((1., 99.)),
        _ => None,
    }
}

fn item_patch(field: &str, value: &str) -> Result<ItemParamPatch> {
    if let Some(transform) = project::TransformField::ALL
        .into_iter()
        .find(|f| f.builtin().as_str() == field)
    {
        let value = value
            .trim()
            .parse::<f64>()
            .map_err(|_| Error::Invalid("transform value"))?;
        let value = transform.clamp(value)?;
        use project::TransformField as F;
        return Ok(match transform {
            F::PositionX => ItemParamPatch::PositionX(value),
            F::PositionY => ItemParamPatch::PositionY(value),
            F::ScaleX => ItemParamPatch::ScaleX(value),
            F::ScaleY => ItemParamPatch::ScaleY(value),
            F::Rotate => ItemParamPatch::Rotate(value),
        });
    }
    Ok(match field {
        "blendMode" => ItemParamPatch::BlendMode(
            serde_json::from_value(serde_json::json!(value))
                .map_err(|_| Error::Invalid("blend mode"))?,
        ),
        _ if project::BuiltinPath::parse(field) == Some(project::BuiltinPath::Opacity) => {
            ItemParamPatch::Opacity(
                value
                    .trim()
                    .parse()
                    .map_err(|_| Error::Invalid("opacity"))?,
            )
        }
        "audio_gain" => ItemParamPatch::AudioGain(
            value
                .trim()
                .parse()
                .map_err(|_| Error::Invalid("audio gain"))?,
        ),
        "audio_muted" => {
            let muted = match value.trim() {
                "true" | "1" | "muted" => true,
                "false" | "0" | "unmuted" => false,
                _ => return Err(Error::Invalid("audio mute")),
            };
            ItemParamPatch::AudioMuted(muted)
        }
        "text_content" => ItemParamPatch::TextContent(value.into()),
        "text_font" => {
            let font = value.trim();
            if font.is_empty() || font.len() > 256 {
                return Err(Error::Invalid("font family"));
            }
            ItemParamPatch::TextFont(font.into())
        }
        "text_size" => ItemParamPatch::TextSize(
            value
                .trim()
                .parse()
                .map_err(|_| Error::Invalid("text size"))?,
        ),
        "text_color" => ItemParamPatch::TextColor(parse_color(value)?),
        "text_weight" => ItemParamPatch::TextWeight(
            value
                .trim()
                .parse()
                .map_err(|_| Error::Invalid("text weight"))?,
        ),
        "text_italic" => ItemParamPatch::TextItalic(parse_flag(value, "text style")?),
        "text_align" => ItemParamPatch::TextAlign(parse_word(value, "text alignment")?),
        "text_decoration" => ItemParamPatch::TextDecoration(parse_word(value, "text decoration")?),
        "text_letter_spacing" => ItemParamPatch::TextLetterSpacing(parse_number(value)?),
        "text_line_height" => ItemParamPatch::TextLineHeight(parse_number(value)?),
        "text_background_padding_x" => ItemParamPatch::TextBackgroundPaddingX(parse_number(value)?),
        "text_background_padding_y" => ItemParamPatch::TextBackgroundPaddingY(parse_number(value)?),
        "text_background_offset_x" => ItemParamPatch::TextBackgroundOffsetX(parse_number(value)?),
        "text_background_offset_y" => ItemParamPatch::TextBackgroundOffsetY(parse_number(value)?),
        "text_background_corner_radius" => {
            ItemParamPatch::TextBackgroundCornerRadius(parse_number(value)?)
        }
        "text_background" => {
            let value = value.trim();
            ItemParamPatch::TextBackground(if value.eq_ignore_ascii_case("none") {
                None
            } else {
                Some(project::TextBackground {
                    color: parse_color(value)?,
                    ..Default::default()
                })
            })
        }
        "text_background_color" => ItemParamPatch::TextBackgroundColor(parse_color(value)?),
        "text_position_x" => ItemParamPatch::TextPositionX(
            parse_percent(value).map_err(|_| Error::Invalid("horizontal position"))?,
        ),
        "text_position_y" => ItemParamPatch::TextPositionY(
            parse_percent(value).map_err(|_| Error::Invalid("vertical position"))?,
        ),
        _ => return Err(Error::Invalid("shared field")),
    })
}
fn parse_number(value: &str) -> Result<f32> {
    value
        .trim()
        .parse()
        .map_err(|_| Error::Invalid("text number"))
}
fn parse_flag(value: &str, what: &'static str) -> Result<bool> {
    match value.trim() {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(Error::Invalid(what)),
    }
}
/// A snake_case enum word such as `line_through`.
fn parse_word<T: serde::de::DeserializeOwned>(value: &str, what: &'static str) -> Result<T> {
    serde_json::from_value(serde_json::json!(value.trim())).map_err(|_| Error::Invalid(what))
}
fn parse_color(value: &str) -> Result<project::Color> {
    Ok(render::color::parse_hex(value)?)
}
fn parse_percent(value: &str) -> Result<f32> {
    let value: f32 = value
        .trim()
        .parse()
        .map_err(|_| Error::Invalid("percentage"))?;
    if !value.is_finite() || !(0.0..=100.0).contains(&value) {
        return Err(Error::Invalid("percentage"));
    }
    Ok(value)
}
