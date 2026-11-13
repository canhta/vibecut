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

/// The admitted range of the parameter keyed on `path`, from the engine's parameter table.
pub(super) fn path_bounds(path: &str) -> Option<(f64, f64)> {
    engine::ITEM_PARAMS
        .iter()
        .find(|row| row.path == Some(path))?
        .bounds
        .limits()
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
        ("params.sides", ItemParams::Graphic(project::GraphicParams::Polygon { sides, .. })) => {
            Some(*sides)
        }
        ("params.points", ItemParams::Graphic(project::GraphicParams::Star { points, .. })) => {
            Some(*points)
        }
        ("params.depth", ItemParams::Graphic(project::GraphicParams::Star { depth, .. })) => {
            Some(*depth)
        }
        ("text_size", ItemParams::Text(params)) => Some(f64::from(params.font_size)),
        ("text_letter_spacing", ItemParams::Text(params)) => Some(f64::from(params.letter_spacing)),
        ("text_line_height", ItemParams::Text(params)) => Some(f64::from(params.line_height)),
        // Unset outlines and shadows show the values a first edit would create.
        ("text_stroke_width", ItemParams::Text(params)) => {
            Some(f64::from(params.stroke.clone().unwrap_or_default().width))
        }
        (
            "text_shadow_offset_x" | "text_shadow_offset_y" | "text_shadow_blur",
            ItemParams::Text(params),
        ) => {
            let shadow = params.shadow.clone().unwrap_or_default();
            Some(f64::from(match field {
                "text_shadow_offset_x" => shadow.offset_x,
                "text_shadow_offset_y" => shadow.offset_y,
                _ => shadow.blur,
            }))
        }
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
        ("volume", ItemParams::Audio(params)) => Some(f64::from(params.volume)),
        ("volume", ItemParams::Video(params)) => Some(f64::from(params.volume)),
        (field, content) if fade_edge(field).is_some() => {
            let edge = fade_edge(field)?;
            Some(fade_on(fades_of(content)?, edge).duration.seconds())
        }
        ("speed", ItemParams::Audio(_) | ItemParams::Video(_)) => Some(item.retime.speed.as_f64()),
        _ => None,
    }
}

/// The cleanup chain of an audible item; other kinds carry none.
fn cleanup_of(content: &ItemParams) -> Option<&project::AudioCleanup> {
    match content {
        ItemParams::Audio(params) => Some(&params.cleanup),
        ItemParams::Video(params) => Some(&params.cleanup),
        _ => None,
    }
}

/// The first noise reduction effect of an audible item's cleanup chain, when it has one.
pub(super) fn noise_reduction(
    project: &project::Project,
    item: ItemId,
) -> Option<&project::CleanupEffect> {
    cleanup_of(&find(project, item)?.content)?
        .effects
        .iter()
        .find(|effect| matches!(effect.kind, project::CleanupKind::NoiseReduction { .. }))
}

/// The smallest id above every effect id of an audible item's cleanup chain.
pub(super) fn next_cleanup_effect_id(project: &project::Project, item: ItemId) -> Option<u32> {
    let cleanup = cleanup_of(&find(project, item)?.content)?;
    Some(
        cleanup
            .effects
            .iter()
            .map(|effect| effect.id)
            .max()
            .map_or(1, |id| id + 1),
    )
}

/// The fades of an audible item; other kinds carry none.
fn fades_of(content: &ItemParams) -> Option<&project::Fades> {
    match content {
        ItemParams::Audio(params) => Some(&params.fades),
        ItemParams::Video(params) => Some(&params.fades),
        _ => None,
    }
}

fn fade_on(fades: &project::Fades, edge: project::FadeEdge) -> project::Fade {
    match edge {
        project::FadeEdge::In => fades.fade_in,
        project::FadeEdge::Out => fades.fade_out,
    }
}

/// The fade edge a property field names (`audio_fade_in` / `audio_fade_out`).
pub(super) fn fade_edge(field: &str) -> Option<project::FadeEdge> {
    match field {
        "audio_fade_in" => Some(project::FadeEdge::In),
        "audio_fade_out" => Some(project::FadeEdge::Out),
        _ => None,
    }
}

/// One fade edge of the item, as stored.
pub(super) fn fade_of(
    project: &project::Project,
    item: ItemId,
    edge: project::FadeEdge,
) -> Option<project::Fade> {
    Some(fade_on(fades_of(&find(project, item)?.content)?, edge))
}

/// The longest duration one edge may take in seconds: the clip less the other
/// edge's audible window. `audio.set_fade` clamps to the same room.
pub(super) fn fade_room_seconds(
    project: &project::Project,
    item: ItemId,
    edge: project::FadeEdge,
) -> Option<f64> {
    let found = find(project, item)?;
    let fades = fades_of(&found.content)?;
    let other = match edge {
        project::FadeEdge::In => fades.fade_out,
        project::FadeEdge::Out => fades.fade_in,
    };
    let clip = found.duration().ok()?.seconds();
    Some((clip - other.window().seconds()).max(0.0))
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
        let (min, max) = path_bounds(field)?;
        return match item
            .animations
            .value_at(&project::AnimationPath::new(field).ok()?, at)
            .ok()
            .flatten()
        {
            Some(project::ParamValue::Scalar(v)) => Some(v.clamp(min, max)),
            _ => item_value(project, item.id, field),
        };
    }
    match project::BuiltinPath::parse(field) {
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
    project::AnimationPath::new(field)
        .ok()
        .and_then(|path| item.animations.get(&path))
        .and_then(|param| match param {
            project::Param::Value(channel) => Some(channel),
            project::Param::Color(_) => None,
        })
        .is_some_and(|channel| !channel.keys().is_empty())
}

/// The nearest key times on each side of a time, item-relative. Both sides are
/// `None` when the channel has no key there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Neighbours {
    pub(super) previous: Option<Tick>,
    pub(super) next: Option<Tick>,
}

impl Neighbours {
    /// The closest previous and the closest next across two channels.
    fn merge(self, other: Self) -> Self {
        Self {
            previous: self.previous.max(other.previous),
            next: match (self.next, other.next) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            },
        }
    }
}

fn channel_neighbours(channel: &project::Channel, at: Tick) -> Neighbours {
    let keys = channel.keys();
    Neighbours {
        previous: keys
            .iter()
            .rev()
            .map(|key| key.time)
            .find(|time| *time < at),
        next: keys.iter().map(|key| key.time).find(|time| *time > at),
    }
}

/// The neighbouring keys of one animated parameter; a colour spans its four channels.
pub(super) fn param_neighbours(param: &project::Param, at: Tick) -> Neighbours {
    match param {
        project::Param::Value(channel) => channel_neighbours(channel, at),
        project::Param::Color(_) => {
            use project::ColorComponent as C;
            [C::R, C::G, C::B, C::A]
                .into_iter()
                .filter_map(|component| param.channel(Some(component)).ok())
                .fold(Neighbours::default(), |all, channel| {
                    all.merge(channel_neighbours(channel, at))
                })
        }
    }
}

/// The neighbouring keys across every listed field of one item at an item-relative
/// time (negative before the item starts), so a pair that keys together navigates together.
pub(super) fn neighbour_keys(
    project: &project::Project,
    item: ItemId,
    fields: &[&str],
    at: Tick,
) -> Neighbours {
    let Some(item) = find(project, item) else {
        return Neighbours::default();
    };
    fields.iter().fold(Neighbours::default(), |all, field| {
        let path = project::AnimationPath::new(*field).ok();
        match path.and_then(|path| item.animations.get(&path)) {
            Some(param) => all.merge(param_neighbours(param, at)),
            None => all,
        }
    })
}

/// Whether the field already owns a key at an item-relative time.
pub(super) fn has_key_at(project: &project::Project, item: ItemId, field: &str, at: Tick) -> bool {
    let Some(item) = find(project, item) else {
        return false;
    };
    let path = project::AnimationPath::new(field).ok();
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

/// The clip background of a video item; `None` for any other item.
pub(super) fn clip_background(
    project: &project::Project,
    item: ItemId,
) -> Option<project::ClipBackground> {
    match &find(project, item)?.content {
        ItemParams::Video(params) => Some(params.background),
        _ => None,
    }
}

/// Whether the Background tab applies at `at`: a video clip that does not cover the canvas.
pub(super) fn clip_background_visible(project: &project::Project, item: ItemId, at: Tick) -> bool {
    let Some(found) = find(project, item) else {
        return false;
    };
    matches!(found.content, ItemParams::Video(_))
        && render::arrange::covers_canvas(project.data(), found, at).is_ok_and(|covers| !covers)
}

/// Unlocked video items, in timeline order: the targets of Apply to all.
pub(super) fn unlocked_video_items(project: &project::Project) -> Vec<ItemId> {
    project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .filter(|item| matches!(item.content, ItemParams::Video(_)))
        .map(|item| item.id)
        .filter(|id| !item_locked(project, *id))
        .collect()
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

/// Whether a video or audio item plays its source backwards.
pub(super) fn retime_reversed(project: &project::Project, item: ItemId) -> Option<bool> {
    Some(find(project, item)?.retime.reverse)
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
