//! Applies parameters commands to a candidate project.
use super::{
    BTreeSet, EditError, Item, ItemId, ItemParams, ProjectData, Result, TrimSources, item,
};

pub(super) fn apply_patch(
    target: &mut Item,
    patch: &crate::ItemParamPatch,
    copied_text_bytes: &mut usize,
) -> Result<()> {
    if let Some((field, value)) = patch.transform() {
        target.animations.set(
            project::AnimationPath::new(field.path())?,
            project::Param::new(project::ParamValue::Scalar(value))?,
        )?;
        return Ok(());
    }
    if let crate::ItemParamPatch::TextContent(value) | crate::ItemParamPatch::TextFont(value) =
        patch
    {
        *copied_text_bytes = copied_text_bytes
            .checked_add(value.as_str().len())
            .filter(|bytes| *bytes <= crate::session::wire::MAX_TEXT_BYTES)
            .ok_or(EditError::Limit)?;
    }
    match patch {
        crate::ItemParamPatch::BlendMode(mode) => target.blend_mode = *mode,
        crate::ItemParamPatch::Opacity(value) => {
            target.animations.set(
                project::AnimationPath::new("opacity")?,
                project::Param::new(project::ParamValue::Scalar(*value))?,
            )?;
        }
        crate::ItemParamPatch::AudioGain(v) => {
            match &mut target.content {
                ItemParams::Audio(p) => p.volume = *v,
                ItemParams::Video(p) => p.volume = *v,
                _ => {}
            }
            target.animations.set(
                project::AnimationPath::new("volume")?,
                project::Param::new(project::ParamValue::Scalar(f64::from(*v)))?,
            )?;
        }
        crate::ItemParamPatch::AudioMuted(v) => {
            match &mut target.content {
                ItemParams::Audio(p) => p.muted = *v,
                ItemParams::Video(p) => p.muted = *v,
                _ => {}
            }
            target.animations.set(
                project::AnimationPath::new("muted")?,
                project::Param::new(project::ParamValue::Bool(*v))?,
            )?;
        }
        crate::ItemParamPatch::Crop(crop) => {
            if let ItemParams::Video(p) = &mut target.content {
                p.crop = *crop;
            }
        }
        crate::ItemParamPatch::TextContent(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.content = v.clone();
            }
        }
        crate::ItemParamPatch::TextFont(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.font = v.clone();
            }
        }
        crate::ItemParamPatch::TextWeight(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.weight = *v;
            }
        }
        crate::ItemParamPatch::TextItalic(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.italic = *v;
            }
        }
        crate::ItemParamPatch::TextLetterSpacing(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.letter_spacing = *v;
            }
        }
        crate::ItemParamPatch::TextLineHeight(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.line_height = *v;
            }
        }
        crate::ItemParamPatch::TextBackground(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.background = v.clone();
            }
        }
        crate::ItemParamPatch::TextBackgroundColor(value) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.background.get_or_insert_with(Default::default).color = *value;
            }
        }
        crate::ItemParamPatch::TextAlign(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.align = *v;
            }
        }
        crate::ItemParamPatch::TextDecoration(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.decoration = *v;
            }
        }
        crate::ItemParamPatch::TextBackgroundPaddingX(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.background.get_or_insert_with(Default::default).padding_x = *v;
            }
        }
        crate::ItemParamPatch::TextBackgroundPaddingY(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.background.get_or_insert_with(Default::default).padding_y = *v;
            }
        }
        crate::ItemParamPatch::TextBackgroundOffsetX(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.background.get_or_insert_with(Default::default).offset_x = *v;
            }
        }
        crate::ItemParamPatch::TextBackgroundOffsetY(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.background.get_or_insert_with(Default::default).offset_y = *v;
            }
        }
        crate::ItemParamPatch::TextBackgroundCornerRadius(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.background.get_or_insert_with(Default::default).radius = *v;
            }
        }
        crate::ItemParamPatch::TextPositionX(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.position.x = *v;
            }
        }
        crate::ItemParamPatch::TextPositionY(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.position.y = *v;
            }
        }
        crate::ItemParamPatch::TextSize(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.font_size = *v;
            }
        }
        crate::ItemParamPatch::TextColor(v) => {
            if let ItemParams::Text(p) = &mut target.content {
                p.color = *v;
            }
        }
        crate::ItemParamPatch::GraphicFill(v) => {
            if let ItemParams::Overlay(p) = &mut target.content {
                p.color = *v;
            }
        }
        crate::ItemParamPatch::GraphicStroke(v) => {
            let [r, g, b, a] = v.0;
            target.animations.set(
                project::AnimationPath::new("params.strokeColor")?,
                project::Param::new(project::ParamValue::Text(
                    format!("#{r:02X}{g:02X}{b:02X}{a:02X}").into(),
                ))?,
            )?;
        }
        _ => return Err(EditError::Invalid("item patch")),
    }
    Ok(())
}
pub(super) fn execute_set_item_param(
    mut data: ProjectData,
    id: ItemId,
    content: ItemParams,
    sources: &mut TrimSources,
) -> Result<ProjectData> {
    item(&mut data, id)?.content = content;
    // Explicit content replacement becomes authoritative for later previews.
    sources.remove(&id);

    Ok(data)
}
pub(super) fn execute_set_item_field(
    mut data: ProjectData,
    id: ItemId,
    patch: crate::ItemParamPatch,
    copied_text_bytes: &mut usize,
) -> Result<ProjectData> {
    patch.validate()?;
    let target = item(&mut data, id)?;
    if !patch.compatible(&target.content) {
        return Err(EditError::Invalid("incompatible item parameter"));
    }
    apply_patch(target, &patch, copied_text_bytes)?;

    Ok(data)
}
pub(super) fn execute_set_item_param_many(
    mut data: ProjectData,
    items: Vec<ItemId>,
    patch: crate::ItemParamPatch,
    copied_text_bytes: &mut usize,
) -> Result<ProjectData> {
    patch.validate()?;
    if items.is_empty() {
        return Err(EditError::Invalid("empty item selection"));
    }
    let mut remaining: BTreeSet<_> = items.iter().copied().collect();
    if remaining.len() != items.len() {
        return Err(EditError::Invalid("duplicate item selection"));
    }
    // Walk the candidate once: large selections must not rescan the timeline per ID.
    for track in data.scene_mut().timeline.iter_mut() {
        for target in &mut track.items {
            if !remaining.remove(&target.id) || !patch.compatible(&target.content) {
                continue;
            }
            if track.locked {
                return Err(EditError::Locked(track.id));
            }
            apply_patch(target, &patch, copied_text_bytes)?;
        }
    }
    if let Some(id) = remaining.into_iter().next() {
        return Err(EditError::NoItem(id));
    }

    Ok(data)
}
