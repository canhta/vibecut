//! Applies parameters commands to a candidate project.
use super::{
    BTreeSet, EditError, Item, ItemId, ItemParams, ProjectData, Result, TrimSources, item,
};
use std::collections::BTreeMap;
use time::Tick;

/// Canvas and source pixel sizes a fit change needs to record the computed fit
/// into a transform; read once before the candidate is borrowed for writing.
pub(in crate::session) struct FitSizes {
    canvas: [f64; 2],
    sources: BTreeMap<ItemId, [f64; 2]>,
}
impl FitSizes {
    fn new(data: &ProjectData, patch: &crate::ItemParamPatch, ids: &[ItemId]) -> Self {
        let mut sources = BTreeMap::new();
        if matches!(patch, crate::ItemParamPatch::Fit(_)) {
            for track in data.scene().timeline.iter() {
                for item in track.items.iter().filter(|item| ids.contains(&item.id)) {
                    if let Some(size) = project::cropped_source_size(&data.assets, item) {
                        sources.insert(item.id, size);
                    }
                }
            }
        }
        Self {
            canvas: [f64::from(data.canvas.width), f64::from(data.canvas.height)],
            sources,
        }
    }
}

/// Switches a video item to the Manual fit, first multiplying the computed fit
/// extent into its static scale so the picture does not move; a canvas resize
/// then no longer re-fits it. A keyed or out-of-range scale refuses; other
/// kinds and an item already Manual are left alone. A source of unknown size
/// has extent 1, as the renderer assumes.
fn enter_manual(target: &mut Item, sizes: &FitSizes) -> Result<()> {
    let ItemParams::Video(params) = &target.content else {
        return Ok(());
    };
    if params.fit == project::FitMode::Manual {
        return Ok(());
    }
    let extent = sizes
        .sources
        .get(&target.id)
        .and_then(|source| params.fit.extent(sizes.canvas, *source))
        .unwrap_or([1.0, 1.0]);
    let mut recorded = Vec::new();
    for (field, factor) in [
        (project::TransformField::ScaleX, extent[0]),
        (project::TransformField::ScaleY, extent[1]),
    ] {
        let path = project::AnimationPath::from(field.builtin());
        let base = match target.animations.get(&path) {
            None => field.default_value(),
            Some(project::Param::Value(channel)) if channel.keys().is_empty() => {
                match channel.base() {
                    project::ParamValue::Scalar(base) => *base,
                    _ => return Err(EditError::Invalid("transform scale")),
                }
            }
            Some(_) => return Err(EditError::Invalid("animated scale cannot record the fit")),
        };
        let scaled = base * factor;
        if field.clamp(scaled)? != scaled {
            return Err(EditError::Invalid("transform bounds"));
        }
        if (factor - 1.0).abs() > f64::EPSILON {
            recorded.push((path, scaled));
        }
    }
    for (path, scaled) in recorded {
        target.animations.set(
            path,
            project::Param::new(project::ParamValue::Scalar(scaled))?,
        )?;
    }
    if let ItemParams::Video(params) = &mut target.content {
        params.fit = project::FitMode::Manual;
    }
    Ok(())
}

/// What applying a patch to one item needs beyond the item: the fit sizes read before the
/// candidate was borrowed, and the running total of text a copy brings in.
pub(in crate::session) struct ApplyCx<'a> {
    sizes: &'a FitSizes,
    copied_text_bytes: &'a mut usize,
}
impl<'a> ApplyCx<'a> {
    fn new(sizes: &'a FitSizes, copied_text_bytes: &'a mut usize) -> Self {
        Self {
            sizes,
            copied_text_bytes,
        }
    }
    /// Sets a video item's fit; Manual first records the computed fit into its scale.
    pub(in crate::session) fn fit(&self, target: &mut Item, mode: project::FitMode) -> Result<()> {
        if mode == project::FitMode::Manual {
            enter_manual(target, self.sizes)?;
        } else if let ItemParams::Video(params) = &mut target.content {
            params.fit = mode;
        }
        Ok(())
    }
    /// Counts text bytes against the per-session copy limit.
    pub(in crate::session) fn charge_text(&mut self, bytes: usize) -> Result<()> {
        *self.copied_text_bytes = self
            .copied_text_bytes
            .checked_add(bytes)
            .filter(|bytes| *bytes <= crate::session::wire::MAX_TEXT_BYTES)
            .ok_or(EditError::Control(
                crate::session::wire::ControlError::Limit,
            ))?;
        Ok(())
    }
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
    let sizes = FitSizes::new(&data, &patch, &[id]);
    let target = item(&mut data, id)?;
    if !patch.compatible(&target.content) {
        return Err(EditError::Invalid("incompatible item parameter"));
    }
    patch.apply(target, &mut ApplyCx::new(&sizes, copied_text_bytes))?;

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
    let sizes = FitSizes::new(&data, &patch, &items);
    let mut cx = ApplyCx::new(&sizes, copied_text_bytes);
    // Walk the candidate once: large selections must not rescan the timeline per ID.
    for track in data.scene_mut().timeline.iter_mut() {
        for target in &mut track.items {
            if !remaining.remove(&target.id) || !patch.compatible(&target.content) {
                continue;
            }
            if track.locked {
                return Err(EditError::Locked(track.id));
            }
            patch.apply(target, &mut cx)?;
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
