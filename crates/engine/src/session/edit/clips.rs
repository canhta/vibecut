//! Applies clips commands to a candidate project.
use super::{
    Arc, BTreeSet, CreatedId, EditError, Item, ItemId, ItemParams, Move, NewItem, Placement,
    ProjectData, Result, SourceRange, Tick, TrackId, TrimSources, alloc, fades_mut, item,
    placement_track, ratchet_fps, sub, take_item, track, unlocked,
};

pub(super) fn move_items(data: &mut ProjectData, moves: Vec<Move>) -> Result<()> {
    let mut seen = BTreeSet::new();
    let mut items = Vec::with_capacity(moves.len());
    for m in moves {
        if !seen.insert(m.item) {
            return Err(EditError::Invalid("duplicate move item"));
        }
        let mut item = take_item(data, m.item)?;
        item.start = m.start;
        items.push((m.track, item));
    }
    let main = data.scene().timeline.main.id;
    let shift = items
        .iter()
        .filter(|(id, _)| *id == main)
        .map(|(_, item)| item.start)
        .min()
        .map(|start| sub(start, placement_start(&data.scene().timeline, main, start)))
        .transpose()?
        .unwrap_or(Tick::ZERO);
    for (id, mut item) in items {
        if id == main {
            item.start = sub(item.start, shift)?;
        }
        let dest = track(data, id)?;
        unlocked(dest)?;
        dest.items.push(item);
    }
    Ok(())
}
// Preserve negative input for validation; first placement onto empty main has no leading gap.
pub(super) fn placement_start(timeline: &project::Timeline, target: TrackId, start: Tick) -> Tick {
    if target == timeline.main.id && timeline.main.items.is_empty() && start > Tick::ZERO {
        Tick::ZERO
    } else {
        start
    }
}

/// Keep caption clipping with its mapping owner, retaining source cue identity and text.
/// The caller supplies authoritative cues, never a previously clipped preview.
pub(super) fn set_range(
    item: &mut Item,
    range: SourceRange,
    key_shift: Tick,
    sources: &mut TrimSources,
) -> Result<()> {
    if let ItemParams::Caption(p) = &mut item.content {
        let mapping = captions::EditMapping::new(vec![(range, range.start())])?;
        let source = sources
            .entry(item.id)
            .or_insert_with(|| Arc::new(p.cues.clone()));
        let mut cues = mapping.map_cues(source)?;
        for cue in &mut cues.cues {
            cue.id = captions::CueFragmentId::from_id(&cue.id)?.source_id;
        }
        p.cues = cues;
    }
    // Keys are item-relative output ticks: they shift by the start edge's output move.
    item.animations.shift(key_shift)?;
    item.source_range = range;
    Ok(())
}
pub(super) fn execute_insert_item(
    mut data: ProjectData,
    placement: Placement,
    new: NewItem,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
) -> Result<ProjectData> {
    if new.start < Tick::ZERO {
        return Err(EditError::Invalid("negative output start"));
    }
    if matches!(new.content, ItemParams::Video(_))
        && let Some(asset) = new
            .source_asset
            .and_then(|id| data.assets.iter().find(|a| a.id() == id))
    {
        ratchet_fps(&mut data.canvas, asset);
        if data.original_canvas_size.is_none()
            && data
                .scenes
                .iter()
                .all(|scene| scene.timeline.iter().all(|track| track.items.is_empty()))
            && let Some(video) = asset
                .probe()
                .streams
                .iter()
                .find_map(|stream| stream.video.as_ref())
        {
            let size = project::CanvasSize {
                width: video.width,
                height: video.height,
            };
            data.canvas.width = size.width;
            data.canvas.height = size.height;
            data.canvas.preset = Some(project::AspectPreset::Original);
            data.canvas.last_custom_size = size;
            data.original_canvas_size = Some(size);
        }
    }
    let kind = new.content.kind();
    let (target, magnetic) = placement_track(
        &mut data,
        placement,
        kind,
        new.start,
        new.retime
            .speed
            .output_duration(new.source_range.duration())?,
        created,
        next,
    )?;
    let start = placement_start(&data.scene().timeline, target, new.start);
    let t = track(&mut data, target)?;
    unlocked(t)?;
    if t.kind != kind {
        return Err(EditError::Invalid("incompatible track"));
    }
    let id = ItemId(alloc(next)?);
    let index = if magnetic {
        // Choose the nearest edge of the clip under the pointer; ties go before.
        let mut index = t.items.len();
        for (i, item) in t.items.iter().enumerate() {
            let end = item.end()?;
            if start < end {
                index = if start <= item.start || sub(start, item.start)? <= sub(end, start)? {
                    i
                } else {
                    i + 1
                };
                break;
            }
        }
        index
    } else {
        t.items.partition_point(|item| item.start <= start)
    };
    t.items.insert(
        index,
        Item {
            blend_mode: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: new.animations,
            id,
            source_asset: new.source_asset,
            source_range: new.source_range,
            start,
            retime: new.retime,
            content: new.content,
        },
    );
    if magnetic {
        let mut at = Tick::ZERO;
        for item in &mut t.items {
            item.start = at;
            at = item.end()?;
        }
    }
    created.push(CreatedId::Item(id));

    Ok(data)
}
pub(super) fn execute_move_item(
    mut data: ProjectData,
    item: ItemId,
    track: TrackId,
    start: Tick,
) -> Result<ProjectData> {
    move_items(&mut data, vec![Move { item, track, start }])?;

    Ok(data)
}
pub(super) fn execute_move_items(mut data: ProjectData, moves: Vec<Move>) -> Result<ProjectData> {
    move_items(&mut data, moves)?;
    Ok(data)
}
pub(super) fn execute_split_at(
    mut data: ProjectData,
    id: ItemId,
    at: Tick,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
    sources: &mut TrimSources,
) -> Result<ProjectData> {
    let left = item(&mut data, id)?;
    let (start, end_tick) = (left.start, left.end()?);
    let (left_cut, right_cut) = left.split_at(at)?;
    let mut right = left.clone();
    set_range(left, left_cut.source_range, Tick::ZERO, sources)?;
    left.retime = left_cut.retime;
    let shift = sub(right.start, right_cut.start)?;
    set_range(&mut right, right_cut.source_range, shift, sources)?;
    right.retime = right_cut.retime;
    // A split establishes independent retained content for both new pieces.
    if let ItemParams::Caption(p) = &left.content {
        sources.insert(left.id, Arc::new(p.cues.clone()));
    }
    // The cut is non-destructive: both pieces keep the whole shape of any fade reaching
    // them, so together they replay the original gain at every sample.
    if let (Some(l), Some(r)) = (fades_mut(&mut left.content), fades_mut(&mut right.content)) {
        (*l, *r) = l.split(start, right_cut.start, end_tick)?;
    }
    right.id = ItemId(alloc(next)?);
    right.start = right_cut.start;
    if let ItemParams::Caption(p) = &right.content {
        sources.insert(right.id, Arc::new(p.cues.clone()));
    }
    created.push(CreatedId::Item(right.id));
    let t = data
        .scene_mut()
        .timeline
        .iter_mut()
        .find(|t| t.items.iter().any(|i| i.id == id))
        .ok_or(EditError::NoItem(id))?;
    let right_id = right.id;
    t.items.push(right);
    for tr in &mut data.scene_mut().timeline.transitions {
        tr.split_piece(id, right_id);
    }

    Ok(data)
}
pub(super) fn execute_trim_start(
    mut data: ProjectData,
    id: ItemId,
    at: Tick,
    sources: &mut TrimSources,
) -> Result<ProjectData> {
    let item = item(&mut data, id)?;
    let moved = item.start != at;
    let cut = item.trim_start(at)?;
    let shift = sub(item.start, cut.start)?;
    set_range(item, cut.source_range, shift, sources)?;
    item.retime = cut.retime;
    item.start = cut.start;
    let clip = item.duration()?;
    if moved && let Some(fades) = fades_mut(&mut item.content) {
        let curve = fades.fade_in.curve;
        let window = fades.fade_in.window();
        fades.set(project::FadeEdge::In, window, curve, clip)?;
    }

    Ok(data)
}
pub(super) fn execute_trim_end(
    mut data: ProjectData,
    id: ItemId,
    at: Tick,
    sources: &mut TrimSources,
) -> Result<ProjectData> {
    let item = item(&mut data, id)?;
    let moved = item.end()? != at;
    let cut = item.trim_end(at)?;
    set_range(item, cut.source_range, Tick::ZERO, sources)?;
    item.retime = cut.retime;
    let clip = item.duration()?;
    if moved && let Some(fades) = fades_mut(&mut item.content) {
        let curve = fades.fade_out.curve;
        let window = fades.fade_out.window();
        fades.set(project::FadeEdge::Out, window, curve, clip)?;
    }

    Ok(data)
}
pub(super) fn execute_delete(mut data: ProjectData, items: Vec<ItemId>) -> Result<ProjectData> {
    let mut ids = BTreeSet::new();
    for id in items {
        if !ids.insert(id) {
            return Err(EditError::Invalid("duplicate delete item"));
        }
        take_item(&mut data, id)?;
    }

    Ok(data)
}
pub(super) fn execute_ripple_delete(mut data: ProjectData, id: ItemId) -> Result<ProjectData> {
    let t = data
        .scene_mut()
        .timeline
        .iter_mut()
        .find(|t| t.items.iter().any(|i| i.id == id))
        .ok_or(EditError::NoItem(id))?;
    unlocked(t)?;
    let index = t
        .items
        .iter()
        .position(|i| i.id == id)
        .ok_or(EditError::NoItem(id))?;
    let removed = t.items.remove(index);
    let end = removed.end()?;
    for item in &mut t.items {
        if item.start >= end {
            item.start = sub(item.start, removed.duration()?)?;
        }
    }

    Ok(data)
}
