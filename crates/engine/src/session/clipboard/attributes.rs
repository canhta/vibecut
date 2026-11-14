//! Pastes the attribute groups of one copied item onto other items.
use super::ItemClipboard;
use crate::session::{EditError, Result};
use project::{AnimationPath, BuiltinPath, Item, ItemId, ItemParams, ProjectData, Retime};
use std::collections::BTreeSet;

/// One group of item attributes that Paste attributes carries as a unit. Clip
/// animations (In, Out, Combo) are ordinary keys on the transform and opacity
/// channels, so they travel with [`Self::Transform`] and [`Self::Blend`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AttributeGroup {
    /// Position, scale, rotation and reflection, with their keyframes.
    Transform,
    /// Opacity with its keyframes, and the blend mode.
    Blend,
    /// The effect stack with its parameters and keyframes.
    Effects,
    /// The masks with their parameters and keyframes.
    Masks,
    /// The whole retime: speed, pitch handling and direction.
    Speed,
    /// Volume, mute and fades, with their keyframes.
    Audio,
    /// The text style: font, colour, outline, shadow and background; never the words.
    Text,
}
impl AttributeGroup {
    /// Every group, in the order Paste applies them.
    pub const ALL: [Self; 7] = [
        Self::Speed,
        Self::Transform,
        Self::Blend,
        Self::Effects,
        Self::Masks,
        Self::Audio,
        Self::Text,
    ];

    /// Whether an item of this content can hold the group.
    pub fn held_by(self, content: &ItemParams) -> bool {
        match self {
            Self::Transform => content.supports_flip(),
            Self::Masks => content.supports_flip() && !matches!(content, ItemParams::Sticker(_)),
            Self::Blend | Self::Effects => !matches!(content, ItemParams::Audio(_)),
            Self::Speed | Self::Audio => {
                matches!(content, ItemParams::Video(_) | ItemParams::Audio(_))
            }
            Self::Text => matches!(content, ItemParams::Text(_)),
        }
    }
}

/// The group an animation channel belongs to; crop and item-parameter channels
/// belong to none.
fn group_of(path: &AnimationPath) -> Option<AttributeGroup> {
    match path.builtin() {
        Some(
            BuiltinPath::TransformPositionX
            | BuiltinPath::TransformPositionY
            | BuiltinPath::TransformScaleX
            | BuiltinPath::TransformScaleY
            | BuiltinPath::TransformRotate
            | BuiltinPath::TransformFlipX
            | BuiltinPath::TransformFlipY,
        ) => Some(AttributeGroup::Transform),
        Some(BuiltinPath::Opacity) => Some(AttributeGroup::Blend),
        Some(BuiltinPath::Blur) => Some(AttributeGroup::Effects),
        Some(BuiltinPath::Volume | BuiltinPath::Muted) => Some(AttributeGroup::Audio),
        Some(
            BuiltinPath::Color
            | BuiltinPath::StrokeColor
            | BuiltinPath::BackgroundColor
            | BuiltinPath::BackgroundPaddingX
            | BuiltinPath::BackgroundPaddingY
            | BuiltinPath::BackgroundOffsetX
            | BuiltinPath::BackgroundOffsetY
            | BuiltinPath::BackgroundCornerRadius,
        ) => Some(AttributeGroup::Text),
        Some(_) => None,
        None if path.as_str().starts_with("effects.") => Some(AttributeGroup::Effects),
        None if path.as_str().starts_with("masks.") => Some(AttributeGroup::Masks),
        None => None,
    }
}

fn audio_of(content: &ItemParams) -> Option<(f32, bool, project::Fades)> {
    match content {
        ItemParams::Video(p) => Some((p.volume, p.muted, p.fades)),
        ItemParams::Audio(p) => Some((p.volume, p.muted, p.fades)),
        _ => None,
    }
}

fn set_audio(content: &mut ItemParams, (volume, muted, fades): (f32, bool, project::Fades)) {
    match content {
        ItemParams::Video(p) => {
            p.volume = volume;
            p.muted = muted;
            p.fades = fades;
        }
        ItemParams::Audio(p) => {
            p.volume = volume;
            p.muted = muted;
            p.fades = fades;
        }
        _ => {}
    }
}

/// Replace the target's channels of `group` with the source's, keys unchanged: key times
/// are item-local, so they stay relative to the target's start.
fn copy_animations(group: AttributeGroup, source: &Item, target: &mut Item) -> Result<()> {
    let stale: Vec<AnimationPath> = target
        .animations
        .iter()
        .filter(|(path, _)| group_of(path) == Some(group))
        .map(|(path, _)| path.clone())
        .collect();
    for path in &stale {
        target.animations.remove(path);
    }
    for (path, param) in source
        .animations
        .iter()
        .filter(|(path, _)| group_of(path) == Some(group))
    {
        target.animations.set(path.clone(), param.clone())?;
    }
    Ok(())
}

/// Copy one group other than Speed, which needs the timeline and so runs apart.
fn apply(group: AttributeGroup, source: &Item, target: &mut Item) -> Result<()> {
    match group {
        AttributeGroup::Speed => return Ok(()),
        AttributeGroup::Transform => {}
        AttributeGroup::Blend => target.blend_mode = source.blend_mode,
        AttributeGroup::Effects => target.effects.clone_from(&source.effects),
        AttributeGroup::Masks => target.masks.clone_from(&source.masks),
        AttributeGroup::Audio => {
            if let Some(audio) = audio_of(&source.content) {
                set_audio(&mut target.content, audio);
            }
        }
        AttributeGroup::Text => {
            if let (ItemParams::Text(from), ItemParams::Text(to)) =
                (&source.content, &mut target.content)
            {
                project::TextStyle::from_params(from).apply_to(to);
            }
        }
    }
    copy_animations(group, source, target)
}

fn target_mut(data: &mut ProjectData, id: ItemId) -> Result<&mut Item> {
    for track in data.scene_mut().timeline.iter_mut() {
        if track.items.iter().any(|item| item.id == id) {
            if track.locked {
                return Err(EditError::Locked(track.id));
            }
            return track
                .items
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or(EditError::NoItem(id));
        }
    }
    Err(EditError::NoItem(id))
}

fn find(data: &ProjectData, id: ItemId) -> Result<&Item> {
    data.scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|item| item.id == id)
        .ok_or(EditError::NoItem(id))
}

/// Groups the copied item holds, in application order, split by whether `content` can hold them.
fn split(source: &Item, content: &ItemParams) -> (Vec<AttributeGroup>, Vec<AttributeGroup>) {
    AttributeGroup::ALL
        .into_iter()
        .filter(|group| group.held_by(&source.content))
        .partition(|group| group.held_by(content))
}

impl ItemClipboard {
    /// The one copied item attributes come from; a multi-item or mask-point snapshot refuses.
    pub(in crate::session) fn attribute_source(&self) -> Result<&Item> {
        match (&self.mask_points, self.items.as_slice()) {
            (None, [entry]) => Ok(&entry.item),
            _ => Err(EditError::Invalid("attributes come from one copied item")),
        }
    }

    /// Per target, the groups the copied item holds that the target kind cannot; a target
    /// that loses nothing is left out. Paste skips these and the status line names them.
    pub(in crate::session) fn attribute_skips(
        &self,
        data: &ProjectData,
        items: &[ItemId],
    ) -> Result<Vec<(ItemId, Vec<AttributeGroup>)>> {
        let source = self.attribute_source()?;
        let mut skips = Vec::new();
        for id in items {
            let (_, skipped) = split(source, &find(data, *id)?.content);
            if !skipped.is_empty() {
                skips.push((*id, skipped));
            }
        }
        Ok(skips)
    }

    /// Whether Paste attributes would admit every target: a copied item, existing
    /// targets on unlocked tracks.
    pub(in crate::session) fn can_paste_attributes(
        &self,
        data: &ProjectData,
        items: &[ItemId],
    ) -> bool {
        !items.is_empty()
            && self.attribute_source().is_ok()
            && items.iter().all(|id| {
                data.scene()
                    .timeline
                    .iter()
                    .any(|track| !track.locked && track.items.iter().any(|item| item.id == *id))
            })
    }
}

/// Give every target the attribute groups of the copied item, skipping the groups a
/// target kind cannot hold. `set_retime` applies a whole [`Retime`] to one item with the
/// timeline's own speed rules; the groups' other values are plain model values.
pub(in crate::session) fn paste_attributes(
    mut data: ProjectData,
    items: &[ItemId],
    payload: Option<&ItemClipboard>,
    set_retime: fn(&mut ProjectData, ItemId, Retime) -> Result<()>,
) -> Result<ProjectData> {
    if items.is_empty() {
        return Err(EditError::Invalid("empty item selection"));
    }
    let mut seen = BTreeSet::new();
    if !items.iter().all(|id| seen.insert(*id)) {
        return Err(EditError::Invalid("duplicate attribute target"));
    }
    let Some(payload) = payload else {
        return Ok(data);
    };
    let source = payload.attribute_source()?;
    for id in items {
        let (applied, _) = split(source, &target_mut(&mut data, *id)?.content);
        // Speed first: a changed speed stretches the target's keys, which the groups
        // below then replace with the copied ones.
        if applied.contains(&AttributeGroup::Speed) && find(&data, *id)?.retime != source.retime {
            set_retime(&mut data, *id, source.retime)?;
        }
        let target = target_mut(&mut data, *id)?;
        for group in applied {
            apply(group, source, target)?;
        }
    }
    Ok(data)
}
