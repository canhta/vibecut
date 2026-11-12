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
        .find(|f| f.builtin().as_str() == field)
    {
        return transform.value_at(&item.animations, Tick::ZERO).ok();
    }
    match (field, &item.content) {
        ("params.strokeWidth", ItemParams::Graphic(p)) => Some(p.appearance().2),
        (
            "params.cornerRadius",
            ItemParams::Graphic(
                project::GraphicParams::Rectangle { corner_radius, .. }
                | project::GraphicParams::Polygon { corner_radius, .. },
            ),
        ) => Some(*corner_radius),
        (
            "params.sides",
            ItemParams::Graphic(project::GraphicParams::Polygon { sides, .. }),
        ) => Some(*sides),
        (
            "params.points",
            ItemParams::Graphic(project::GraphicParams::Star { points, .. }),
        ) => Some(*points),
        ("params.depth", ItemParams::Graphic(project::GraphicParams::Star { depth, .. })) => {
            Some(*depth)
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

/// The field's model value at an item-relative time, for keyframe toggles.
pub(super) fn item_value_at(
    project: &project::Project,
    item: ItemId,
    field: &str,
    at: Tick,
) -> Option<f64> {
    let item = find(project, item)?;
    if let Some(transform) = project::TransformField::ALL
        .into_iter()
        .find(|f| f.builtin().as_str() == field)
    {
        return transform.value_at(&item.animations, at).ok();
    }
    if field.starts_with("params.") && matches!(item.content, ItemParams::Graphic(_)) {
        let max = match field {
            "params.strokeWidth" => 64.,
            "params.depth" => 99.,
            "params.sides" | "params.points" => 12.,
            _ => 50.,
        };
        return match item
            .animations
            .value_at(&project::AnimationPath::new(field).ok()?, at)
            .ok()
            .flatten()
        {
            Some(project::ParamValue::Scalar(v)) => Some(v.clamp(0., max)),
            _ => item_value(project, item.id, field),
        };
    }
    let builtin = if field == "audio_gain" {
        Some(project::BuiltinPath::Volume)
    } else {
        project::BuiltinPath::parse(field)
    };
    match builtin {
        Some(project::BuiltinPath::Opacity) if !matches!(&item.content, ItemParams::Audio(_)) => {
            let path = project::AnimationPath::from(project::BuiltinPath::Opacity);
            return Some(match item.animations.value_at(&path, at).ok().flatten() {
                Some(project::ParamValue::Scalar(value)) => value.clamp(0.0, 1.0),
                _ => 1.0,
            });
        }
        Some(project::BuiltinPath::Volume) => {
            let base = match &item.content {
                ItemParams::Audio(params) => params.volume,
                ItemParams::Video(params) => params.volume,
                _ => return None,
            };
            let path = project::AnimationPath::from(project::BuiltinPath::Volume);
            return Some(match item.animations.value_at(&path, at).ok().flatten() {
                Some(project::ParamValue::Scalar(value)) => value,
                _ => f64::from(base),
            });
        }
        _ => {}
    }
    if let Some(rest) = field.strip_prefix("effects.")
        && let Some((effect, key)) = rest.split_once(".params.")
    {
        return effect_param_value_at(project, item.id, effect, key, at);
    }
    item_value(project, item.id, field)
}

/// Whether the field owns any keys, so a field edit must update the curve.
pub(super) fn channel_has_keys(project: &project::Project, item: ItemId, field: &str) -> bool {
    let Some(item) = find(project, item) else {
        return false;
    };
    let path = if field == "audio_gain" {
        Some(project::AnimationPath::from(project::BuiltinPath::Volume))
    } else {
        project::AnimationPath::new(field).ok()
    };
    path.and_then(|path| item.animations.get(&path))
        .and_then(|param| match param {
            project::Param::Value(channel) => Some(channel),
            project::Param::Color(_) => None,
        })
        .is_some_and(|channel| !channel.keys().is_empty())
}

/// Whether the field already owns a key at an item-relative time.
pub(super) fn has_key_at(project: &project::Project, item: ItemId, field: &str, at: Tick) -> bool {
    let Some(item) = find(project, item) else {
        return false;
    };
    let path = if field == "audio_gain" {
        Some(project::AnimationPath::from(project::BuiltinPath::Volume))
    } else {
        project::AnimationPath::new(field).ok()
    };
    let Some(path) = path else {
        return false;
    };
    item.animations
        .get(&path)
        .and_then(|param| match param {
            project::Param::Value(channel) => Some(channel),
            project::Param::Color(_) => None,
        })
        .is_some_and(|channel| {
            channel
                .keys()
                .binary_search_by_key(&at, |key| key.time)
                .is_ok()
        })
}

/// Whether the item's track refuses edits.
pub(super) fn item_locked(project: &project::Project, id: ItemId) -> bool {
    project
        .data()
        .scene()
        .timeline
        .iter()
        .any(|track| track.locked && track.items.iter().any(|item| item.id == id))
}

/// The item's start and end on the timeline.
pub(super) fn range_of(project: &project::Project, id: ItemId) -> Option<(Tick, Tick)> {
    let item = find(project, id)?;
    Some((item.start, item.end().ok()?))
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

/// The effect scalar at an item-relative time: an animated channel reads its
/// curve, everything else its persisted value.
pub(super) fn effect_param_value_at(
    project: &project::Project,
    item: ItemId,
    effect: &str,
    key: &str,
    at: Tick,
) -> Option<f64> {
    let item = find(project, item)?;
    let instance = item
        .effects
        .iter()
        .find(|candidate| candidate.id == effect)?;
    let base = instance.value(key)?.scalar()?;
    let path = project::AnimationPath::new(format!("effects.{effect}.params.{key}")).ok()?;
    Some(match item.animations.value_at(&path, at).ok().flatten() {
        Some(project::ParamValue::Scalar(value)) => value,
        _ => base,
    })
}

/// One static tone curve owned by an effect instance.
pub(super) fn effect_curve(
    project: &project::Project,
    item: ItemId,
    effect: &str,
    key: &str,
) -> Option<project::ToneCurve> {
    find(project, item)?
        .effects
        .iter()
        .find(|candidate| candidate.id == effect)?
        .value(key)?
        .curve()
        .copied()
}

/// The clip's chroma-key instance, the one the Remove BG sub-tab edits.
pub(super) fn chroma_key(
    project: &project::Project,
    item: ItemId,
) -> Option<&project::EffectInstance> {
    find(project, item)?
        .effects
        .iter()
        .find(|candidate| candidate.kind == "chroma_key")
}

/// An instance's key colour, opaque.
pub(super) fn chroma_key_color(
    project: &project::Project,
    item: ItemId,
    effect: &str,
) -> Option<project::Color> {
    find(project, item)?
        .effects
        .iter()
        .find(|candidate| candidate.id == effect && candidate.kind == "chroma_key")?
        .value("key_color")?
        .color()
}
