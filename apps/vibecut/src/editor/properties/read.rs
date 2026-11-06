//! Projects values the panel displays out of the immutable project.
use super::{ItemId, ItemParams, Tick};

fn find(project: &project::Project, id: ItemId) -> Option<&project::Item> {
    project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|candidate| candidate.id == id)
}

pub(super) fn item(project: &project::Project, id: ItemId) -> Option<&project::Item> {
    find(project, id)
}

/// The field's model value at the start of the item, for display.
pub(super) fn item_value(project: &project::Project, item: ItemId, field: &str) -> Option<f64> {
    let item = find(project, item)?;
    if let Some(transform) = project::TransformField::ALL
        .into_iter()
        .find(|f| f.path() == field)
    {
        return transform.value_at(&item.animations, Tick::ZERO).ok();
    }
    match (field, &item.content) {
        ("opacity", content) if !matches!(content, ItemParams::Audio(_)) => {
            let path = project::AnimationPath::new("opacity").ok()?;
            Some(
                match item.animations.value_at(&path, Tick::ZERO).ok().flatten() {
                    Some(project::ParamValue::Scalar(value)) => value.clamp(0.0, 1.0),
                    _ => 1.0,
                },
            )
        }
        ("text_size", ItemParams::Text(params)) => Some(f64::from(params.font_size)),
        ("text_letter_spacing", ItemParams::Text(params)) => Some(f64::from(params.letter_spacing)),
        ("text_line_height", ItemParams::Text(params)) => Some(f64::from(params.line_height)),
        (field, ItemParams::Text(params)) if field.starts_with("text_background_") => {
            // Unset boxes show the defaults a first edit would create.
            let background = params.background.clone().unwrap_or_default();
            Some(f64::from(match field {
                "text_background_padding_x" => background.padding_x,
                "text_background_padding_y" => background.padding_y,
                "text_background_offset_x" => background.offset_x,
                "text_background_offset_y" => background.offset_y,
                "text_background_corner_radius" => background.radius,
                _ => return None,
            }))
        }
        ("audio_gain", ItemParams::Audio(params)) => Some(f64::from(params.volume)),
        ("audio_gain", ItemParams::Video(params)) => Some(f64::from(params.volume)),
        ("speed", ItemParams::Audio(_) | ItemParams::Video(_)) => Some(item.retime.speed.as_f64()),
        _ => None,
    }
}

/// Source and retimed duration in seconds, for display only.
pub(super) fn retime_seconds(project: &project::Project, item: ItemId) -> Option<(f64, f64)> {
    let found = find(project, item)?;
    Some((
        found.source_range.duration().seconds(),
        found.duration().ok()?.seconds(),
    ))
}

/// Speed, pitch policy and whether the item has audio to retime.
pub(super) fn retime_of(project: &project::Project, item: ItemId) -> Option<(f64, bool, bool)> {
    let found = find(project, item)?;
    let audible = match &found.content {
        ItemParams::Audio(_) => true,
        ItemParams::Video(params) => params.source_audio_enabled,
        _ => return None,
    };
    Some((
        found.retime.speed.as_f64(),
        found.retime.maintain_pitch,
        audible,
    ))
}

pub(super) fn effect_param_value(
    project: &project::Project,
    item: ItemId,
    effect: &str,
    key: &str,
) -> Option<f64> {
    let effect = find(project, item)?
        .effects
        .iter()
        .find(|candidate| candidate.id == effect)?;
    match (&effect.params, key) {
        (project::EffectParams::Blur { intensity }, "intensity") => Some(*intensity),
        _ => None,
    }
}
