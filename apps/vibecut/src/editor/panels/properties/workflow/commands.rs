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
    if !desk.editor().text_animation_available(state)
        || state.animation_target(desk.view().session, desk.view().selection) != Some(item)
        || !std::sync::Arc::ptr_eq(&snapshot, desk.view().session.project())
    {
        return Err(Error::Invalid("text animation target changed"));
    }
    desk.end_merge();
    desk.apply_result(engine::Command::ApplyTextAnimation { item, preset }, false)?;
    Ok(())
}

/// The inspector and automation use the same registered atomic field edit.
pub(in crate::editor) fn set_shared_field(
    desk: &mut Desk<'_>,
    field: &str,
    value: &str,
) -> Result<()> {
    if field == "opacity"
        && let Some(item) = crate::editor::context::single(desk.view().selection)
    {
        return set_item_field(desk, item, field, value);
    }
    let patch = item_patch(field, value)?;
    let items = desk.view().selection.iter().copied().collect();
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

/// One element's committed field edit, through the singular registered command.
pub(in crate::editor) fn set_item_field(
    desk: &mut Desk<'_>,
    item: ItemId,
    field: &str,
    value: &str,
) -> Result<()> {
    if field == "opacity" && read::channel_has_keys(desk.view().session.project(), item, field) {
        let (start, end) = read::range_of(desk.view().session.project(), item)
            .ok_or(Error::Invalid("opacity target"))?;
        let at = desk.view().playhead;
        if at < start || at >= end {
            return Err(Error::Invalid("opacity playhead outside item"));
        }
        let value = value
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
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
    let patch = item_patch(field, value)?;
    desk.apply_result(engine::Command::SetItemField { item, patch }, false)?;
    Ok(())
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
