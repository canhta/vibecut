//! Applies parameters commands to a candidate project.
use super::{
    BTreeSet, EditError, Item, ItemId, ItemParams, ProjectData, Result, TrimSources, item,
};
use time::Tick;

pub(super) fn apply_patch(
    target: &mut Item,
    patch: &crate::ItemParamPatch,
    copied_text_bytes: &mut usize,
) -> Result<()> {
    if let Some((field, value)) = patch.transform() {
        target.animations.set(
            project::AnimationPath::from(field.builtin()),
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
            .ok_or(EditError::Control(
                crate::session::wire::ControlError::Limit,
            ))?;
    }
    match patch {
        crate::ItemParamPatch::BlendMode(mode) => target.blend_mode = *mode,
        crate::ItemParamPatch::Opacity(value) => {
            target.animations.set(
                project::AnimationPath::from(project::BuiltinPath::Opacity),
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
                project::AnimationPath::from(project::BuiltinPath::Volume),
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
                project::AnimationPath::from(project::BuiltinPath::Muted),
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
        crate::ItemParamPatch::GraphicFill(v) => match &mut target.content {
            ItemParams::Overlay(p) => p.color = *v,
            ItemParams::Graphic(
                project::GraphicParams::Rectangle { fill, .. }
                | project::GraphicParams::Ellipse { fill, .. }
                | project::GraphicParams::Polygon { fill, .. }
                | project::GraphicParams::Star { fill, .. },
            ) => *fill = *v,
            _ => {}
        },
        crate::ItemParamPatch::GraphicStroke(v) => {
            if let ItemParams::Graphic(
                project::GraphicParams::Rectangle { stroke, .. }
                | project::GraphicParams::Ellipse { stroke, .. }
                | project::GraphicParams::Polygon { stroke, .. }
                | project::GraphicParams::Star { stroke, .. },
            ) = &mut target.content
            {
                *stroke = *v;
                return Ok(());
            }
            let [r, g, b, a] = v.0;
            target.animations.set(
                project::AnimationPath::new("params.strokeColor")?,
                project::Param::new(project::ParamValue::Text(
                    format!("#{r:02X}{g:02X}{b:02X}{a:02X}").into(),
                ))?,
            )?;
        }
        crate::ItemParamPatch::GraphicStrokeWidth(v) => {
            if let ItemParams::Graphic(
                project::GraphicParams::Rectangle { stroke_width, .. }
                | project::GraphicParams::Ellipse { stroke_width, .. }
                | project::GraphicParams::Polygon { stroke_width, .. }
                | project::GraphicParams::Star { stroke_width, .. },
            ) = &mut target.content
            {
                *stroke_width = *v;
            }
        }
        crate::ItemParamPatch::GraphicStrokeAlign(v) => {
            if let ItemParams::Graphic(
                project::GraphicParams::Rectangle { stroke_align, .. }
                | project::GraphicParams::Ellipse { stroke_align, .. }
                | project::GraphicParams::Polygon { stroke_align, .. }
                | project::GraphicParams::Star { stroke_align, .. },
            ) = &mut target.content
            {
                *stroke_align = *v;
            }
        }
        crate::ItemParamPatch::GraphicCornerRadius(v) => {
            if let ItemParams::Graphic(
                project::GraphicParams::Rectangle { corner_radius, .. }
                | project::GraphicParams::Polygon { corner_radius, .. },
            ) = &mut target.content
            {
                *corner_radius = *v;
            }
        }
        crate::ItemParamPatch::GraphicSides(v) => {
            if let ItemParams::Graphic(project::GraphicParams::Polygon { sides, .. }) =
                &mut target.content
            {
                *sides = *v;
            }
        }
        crate::ItemParamPatch::GraphicPoints(v) => {
            if let ItemParams::Graphic(project::GraphicParams::Star { points, .. }) =
                &mut target.content
            {
                *points = *v;
            }
        }
        crate::ItemParamPatch::GraphicDepth(v) => {
            if let ItemParams::Graphic(project::GraphicParams::Star { depth, .. }) =
                &mut target.content
            {
                *depth = *v;
            }
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

/// Set one reflection axis on every listed item as one candidate change. All
/// targets are resolved before any write; an axis carrying keys, a locked track,
/// a missing/other-scene ID or an unsupported kind refuses the whole list.
pub(super) fn execute_set_items_flipped(
    mut data: ProjectData,
    items: Vec<ItemId>,
    axis: project::FlipAxis,
    flipped: bool,
) -> Result<ProjectData> {
    if items.is_empty() {
        return Err(EditError::Invalid("empty item selection"));
    }
    let mut remaining: BTreeSet<_> = items.iter().copied().collect();
    if remaining.len() != items.len() {
        return Err(EditError::Invalid("duplicate item selection"));
    }
    let path = project::AnimationPath::from(axis.builtin());
    for track in data.scene_mut().timeline.iter_mut() {
        for target in &mut track.items {
            if !remaining.remove(&target.id) {
                continue;
            }
            if track.locked {
                return Err(EditError::Locked(track.id));
            }
            if !target.content.supports_flip() {
                return Err(EditError::Invalid("flip on unsupported item"));
            }
            match target.animations.get(&path) {
                Some(param) => {
                    if !param.channel(None)?.keys().is_empty() {
                        return Err(EditError::Invalid("flip axis is keyed"));
                    }
                    match param.value_at(Tick::ZERO)? {
                        project::ParamValue::Bool(current) if current == flipped => continue,
                        project::ParamValue::Bool(_) => {}
                        _ => return Err(EditError::Invalid("flip value")),
                    }
                }
                // An absent channel already means `false`.
                None if !flipped => continue,
                None => {}
            }
            target.animations.set(
                path.clone(),
                project::Param::new(project::ParamValue::Bool(flipped))?,
            )?;
        }
    }
    if let Some(id) = remaining.into_iter().next() {
        return Err(EditError::NoItem(id));
    }

    Ok(data)
}

/// Current crop channel of `path`, if the item has one.
fn crop_channel(item: &Item, path: project::BuiltinPath) -> Option<project::Channel> {
    let param = item.animations.get(&project::AnimationPath::from(path))?;
    param.channel(None).ok().cloned()
}

/// Marks the shot containing `at` as manual by clearing its `crop.auto` flag.
/// Position keys stay exactly as they are; the user edits them with the ordinary
/// keyframe commands.
pub(super) fn execute_reframe_set_manual(
    mut data: ProjectData,
    id: ItemId,
    at: Tick,
) -> Result<ProjectData> {
    let target = item(&mut data, id)?;
    let auto = crop_channel(target, project::BuiltinPath::CropAuto)
        .ok_or(EditError::Invalid("reframe is not generated"))?;
    if at < Tick::ZERO || at >= target.duration()? {
        return Err(EditError::Invalid("reframe shot time"));
    }
    let start = auto
        .keys()
        .iter()
        .rev()
        .find(|key| key.time <= at)
        .map(|key| key.time)
        .ok_or(EditError::Invalid("reframe shot time"))?;
    let mut keys = auto.keys().to_vec();
    let index = keys
        .binary_search_by_key(&start, |key| key.time)
        .map_err(|_| EditError::Invalid("reframe shot"))?;
    let Some(key) = keys.get_mut(index) else {
        return Err(EditError::Invalid("reframe shot"));
    };
    if key.value == project::ParamValue::Bool(false) {
        return Ok(data);
    }
    key.value = project::ParamValue::Bool(false);
    let mut auto = auto;
    auto.put(
        keys.get(index)
            .cloned()
            .ok_or(EditError::Invalid("reframe shot"))?,
    )?;
    target.animations.set(
        project::AnimationPath::from(project::BuiltinPath::CropAuto),
        project::Param::Value(auto),
    )?;
    Ok(data)
}

/// Removes the three crop channels and leaves the static crop untouched.
pub(super) fn execute_reframe_clear(mut data: ProjectData, id: ItemId) -> Result<ProjectData> {
    let target = item(&mut data, id)?;
    for path in [
        project::BuiltinPath::CropX,
        project::BuiltinPath::CropY,
        project::BuiltinPath::CropAuto,
    ] {
        target
            .animations
            .remove(&project::AnimationPath::from(path));
    }
    Ok(data)
}

/// Aligns and evenly spaces selections by translating transform position.
pub(super) mod arrange {
    use super::super::{EditError, ItemId, ProjectData, Result, item};
    use project::{
        AnimationPath, Channel, DistributionAxis, ItemAlignment, Param, ParamValue, TransformField,
    };
    use std::collections::BTreeSet;
    use time::Tick;

    /// The smallest translation treated as a real change, in project pixels.
    const RESIDUE: f64 = 1e-9;

    /// Move every listed item to one canvas edge or centre. Each item is aligned
    /// against the canvas independently, not against the selection envelope.
    pub(in super::super) fn execute_align(
        mut data: ProjectData,
        items: Vec<ItemId>,
        alignment: ItemAlignment,
        at: Tick,
    ) -> Result<ProjectData> {
        let targets = targets(&data, &items, at)?;
        let width = f64::from(data.canvas.width);
        let height = f64::from(data.canvas.height);
        for (id, bounds) in targets {
            let (dx, dy) = match alignment {
                ItemAlignment::Left => (-bounds.x, 0.0),
                ItemAlignment::HorizontalCenter => {
                    (width / 2.0 - (bounds.x + bounds.width / 2.0), 0.0)
                }
                ItemAlignment::Right => (width - (bounds.x + bounds.width), 0.0),
                ItemAlignment::Top => (0.0, -bounds.y),
                ItemAlignment::VerticalCenter => {
                    (0.0, height / 2.0 - (bounds.y + bounds.height / 2.0))
                }
                ItemAlignment::Bottom => (0.0, height - (bounds.y + bounds.height)),
            };
            if dx.abs() > RESIDUE {
                translate(&mut data, id, TransformField::PositionX, dx)?;
            }
            if dy.abs() > RESIDUE {
                translate(&mut data, id, TransformField::PositionY, dy)?;
            }
        }
        Ok(data)
    }

    /// Space three or more listed items evenly along one axis, pinning the first
    /// leading edge and the last trailing edge. The gaps between boxes are equal.
    pub(in super::super) fn execute_distribute(
        mut data: ProjectData,
        items: Vec<ItemId>,
        axis: DistributionAxis,
        at: Tick,
    ) -> Result<ProjectData> {
        let targets = targets(&data, &items, at)?;
        if targets.len() < 3 {
            return Err(EditError::Invalid("distribute needs three items"));
        }
        // Bounds are plain geometry; the item ID breaks every exact tie, so input
        // order never decides which item lands where.
        let horizontal = axis == DistributionAxis::Horizontal;
        let mut sorted = targets
            .into_iter()
            .map(|(id, bounds)| {
                let (leading, extent) = if horizontal {
                    (bounds.x, bounds.width)
                } else {
                    (bounds.y, bounds.height)
                };
                (id, leading, extent)
            })
            .collect::<Vec<_>>();
        sorted.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        let first = sorted.first().map_or(0.0, |target| target.1);
        let last = sorted
            .last()
            .map(|target| target.1 + target.2)
            .unwrap_or(first);
        let total: f64 = sorted.iter().map(|target| target.2).sum();
        let gap = (last - first - total) / (sorted.len() - 1) as f64;
        if !gap.is_finite() || gap < -RESIDUE {
            return Err(EditError::Invalid("items overlap for distribution"));
        }
        let gap = gap.max(0.0);
        let field = if horizontal {
            TransformField::PositionX
        } else {
            TransformField::PositionY
        };
        let mut cursor = first;
        for (index, (id, leading, extent)) in sorted.iter().enumerate() {
            if index > 0 {
                let delta = cursor - *leading;
                if delta.abs() > RESIDUE {
                    translate(&mut data, *id, field, delta)?;
                }
            }
            cursor += *extent + gap;
        }
        Ok(data)
    }

    /// Resolve the whole explicit selection to its canvas envelopes before any
    /// write. Missing, other-scene, duplicate, unsupported or locked targets
    /// refuse the operation without a partial change.
    fn targets(
        data: &ProjectData,
        items: &[ItemId],
        at: Tick,
    ) -> Result<Vec<(ItemId, render::Rect)>> {
        if items.is_empty() {
            return Err(EditError::Invalid("empty item selection"));
        }
        if at < Tick::ZERO {
            return Err(EditError::Invalid("negative arrangement time"));
        }
        let mut remaining: BTreeSet<_> = items.iter().copied().collect();
        if remaining.len() != items.len() {
            return Err(EditError::Invalid("duplicate item selection"));
        }
        let mut targets = Vec::with_capacity(items.len());
        for track in data.scene().timeline.iter() {
            for target in &track.items {
                if !remaining.remove(&target.id) {
                    continue;
                }
                if track.locked {
                    return Err(EditError::Locked(track.id));
                }
                if !target.content.supports_arrange() {
                    return Err(EditError::Invalid("arrange on unsupported item"));
                }
                let bounds = render::arrange::bounds(data, target, at)
                    .map_err(|_| EditError::Invalid("item arrangement geometry"))?;
                targets.push((target.id, bounds));
            }
        }
        if let Some(id) = remaining.into_iter().next() {
            return Err(EditError::NoItem(id));
        }
        Ok(targets)
    }

    /// Translate one position channel by `delta`, moving its base and every scalar
    /// key. Key times, segments, handles and extrapolation are preserved. The raw
    /// and shifted values must both fit the channel's admitted numeric range, so
    /// evaluation clamping can never shorten the requested move.
    fn translate(
        data: &mut ProjectData,
        id: ItemId,
        field: TransformField,
        delta: f64,
    ) -> Result<()> {
        let path = AnimationPath::from(field.builtin());
        let target = item(data, id)?;
        let existing = target.animations.get(&path).cloned();
        let replaced = match existing {
            None => {
                let base = shifted(field.default_value(), delta, field)?;
                Param::new(ParamValue::Scalar(base))?
            }
            Some(param) => {
                let channel = param.channel(None)?;
                let ParamValue::Scalar(base) = channel.base() else {
                    return Err(EditError::Invalid("transform scalar"));
                };
                let base = shifted(*base, delta, field)?;
                let mut keys = Vec::with_capacity(channel.keys().len());
                for key in channel.keys() {
                    let ParamValue::Scalar(value) = &key.value else {
                        return Err(EditError::Invalid("transform scalar"));
                    };
                    let mut key = key.clone();
                    key.value = ParamValue::Scalar(shifted(*value, delta, field)?);
                    keys.push(key);
                }
                let (before, after) = channel.extrapolation();
                Param::Value(Channel::new(ParamValue::Scalar(base), keys, before, after)?)
            }
        };
        target.animations.set(path, replaced)?;
        Ok(())
    }

    /// Reject values outside the transform field's admitted range instead of
    /// clamping them, so a refused move never partially applies.
    fn shifted(value: f64, delta: f64, field: TransformField) -> Result<f64> {
        let (min, max) = field.bounds();
        let shifted = value + delta;
        if !value.is_finite()
            || !shifted.is_finite()
            || !(min..=max).contains(&value)
            || !(min..=max).contains(&shifted)
        {
            return Err(EditError::Invalid("alignment position bound"));
        }
        Ok(shifted)
    }
}
