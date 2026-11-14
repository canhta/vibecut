//! Applies clips commands to a candidate project.
use super::animation;
use super::{
    Arc, BTreeMap, BTreeSet, Command, CreatedId, EditError, Item, ItemId, ItemParams, Move,
    NewItem, Placement, ProjectData, Result, SourceRange, Tick, Track, TrackId, TrackKind,
    TrackPlacement, TrimSources, alloc, fades_mut, item, placement_track, ratchet_fps, sub,
    take_item, track, unlocked,
};
use crate::ClipAnimations;
use project::GroupId;

mod compound;
pub(super) use compound::{admit_content, execute_compound, execute_uncompound};

pub(super) fn move_items(data: &mut ProjectData, moves: Vec<Move>) -> Result<()> {
    let moves = with_group_moves(data, moves)?;
    // Only a first placement onto a main track that was already empty clamps to zero; moving
    // the main track's own items leaves the gap they open at the origin.
    let main_was_empty = data.scene().timeline.main.items.is_empty();
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
        .filter(|(id, _)| main_was_empty && *id == main)
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
        let source = sources
            .entry(item.id)
            .or_insert_with(|| Arc::new(p.cues.clone()));
        p.cues = captions::clip_track(source, range)?;
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
            // Odd media sizes round like custom entry so the canvas can export.
            let raw = project::CanvasSize {
                width: video.width,
                height: video.height,
            };
            let size = raw.rounded_even().filter(|s| s.is_valid()).unwrap_or(raw);
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
            hidden: false,
            blend_mode: Default::default(),
            group: None,
            masks: Vec::new(),
            effects: Vec::new(),
            animations: new.animations,
            id,
            source_asset: new.source_asset,
            source_range: new.source_range,
            start,
            retime: *new.retime,
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
/// Paste copied items at `at`, clearing every overlapped target range first.
/// Each item resolves its source track kind/index, falling back to the last
/// track of that kind; fresh identities and sorted starts keep one undo step.
pub(super) fn execute_paste_items(
    mut data: ProjectData,
    at: Tick,
    onto: Option<TrackId>,
    payload: Option<&crate::ItemClipboard>,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
    sources: &mut TrimSources,
) -> Result<ProjectData> {
    let Some(payload) = payload else {
        return Ok(data);
    };
    let destinations = payload.destinations(&data, at, onto)?;
    if destinations.is_empty() {
        return Ok(data);
    }
    let mut spans: BTreeMap<TrackId, Vec<(Tick, Tick)>> = BTreeMap::new();
    for destination in &destinations {
        spans
            .entry(destination.track)
            .or_default()
            .push((destination.start, destination.end));
    }
    for (track_id, spans) in spans {
        for (start, end) in merge_spans(spans) {
            let ids: Vec<ItemId> = data
                .scene()
                .timeline
                .iter()
                .find(|item_track| item_track.id == track_id)
                .map(|item_track| item_track.items.iter().map(|item| item.id).collect())
                .unwrap_or_default();
            for id in ids {
                data = clear_overlap(data, id, start, end, created, next, sources)?;
            }
        }
    }
    for destination in destinations {
        let id = ItemId(alloc(next)?);
        let mut item = destination.item(id);
        compound::renumber(&mut item, next)?;
        let target = track(&mut data, destination.track)?;
        unlocked(target)?;
        let index = target
            .items
            .partition_point(|existing| existing.start <= item.start);
        target.items.insert(index, item);
        created.push(CreatedId::Item(id));
    }
    Ok(data)
}

fn merge_spans(mut spans: Vec<(Tick, Tick)>) -> Vec<(Tick, Tick)> {
    spans.sort();
    let mut merged: Vec<(Tick, Tick)> = Vec::new();
    for (start, end) in spans {
        match merged.last_mut() {
            Some((_, last_end)) if start <= *last_end => *last_end = (*last_end).max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

/// Clear one existing range that a paste overwrites, splitting a containing
/// item so both surviving heads keep their source, retime, fades and cues.
fn clear_overlap(
    mut data: ProjectData,
    id: ItemId,
    start: Tick,
    end: Tick,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
    sources: &mut TrimSources,
) -> Result<ProjectData> {
    let Some((item_start, item_end)) = find_item(&data, id)
        .map(|(_, item)| item.end().map(|end| (item.start, end)))
        .transpose()?
    else {
        return Ok(data);
    };
    if item_end <= start || item_start >= end {
        return Ok(data);
    }
    if item_start < start && item_end > end {
        let mark = created.len();
        data = execute_split_at(data, id, start, created, next, sources)?;
        let right = match created.get(mark).copied() {
            Some(CreatedId::Item(right)) => right,
            _ => return Err(EditError::Invalid("clipboard overwrite split")),
        };
        created.truncate(mark);
        data = execute_trim_start(data, right, end, sources)?;
    } else if item_start < start {
        data = execute_trim_end(data, id, start, sources)?;
    } else if item_end > end {
        data = execute_trim_start(data, id, end, sources)?;
    } else {
        data = execute_delete(data, vec![id])?;
    }
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

/// Resolve destinations on the candidate before moving the selection simultaneously.
pub(super) fn move_group(
    mut data: ProjectData,
    ids: Vec<ItemId>,
    delta: Tick,
    placement: Option<Placement>,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
) -> Result<ProjectData> {
    let ids = with_group_members(&data, &ids);
    let mut seen = BTreeSet::new();
    let mut members = Vec::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(EditError::Invalid("duplicate group item"));
        }
        let source = data
            .scene()
            .timeline
            .iter()
            .find(|t| t.items.iter().any(|i| i.id == id))
            .ok_or(EditError::NoItem(id))?
            .id;
        members.push((source, item(&mut data, id)?.clone()));
    }
    if members.is_empty() {
        return Err(EditError::Invalid("empty timeline group"));
    }
    // Remove every member first: auto placement must not collide with the old group.
    for (_, member) in &members {
        take_item(&mut data, member.id)?;
    }
    let mut destinations = std::collections::BTreeMap::new();
    let mut moves = Vec::new();
    for (source, member) in &members {
        let start = super::add(member.start, delta)?;
        let target = if let Some(placement) = placement {
            if let Some(target) = destinations.get(source) {
                *target
            } else {
                let (target, _) = placement_track(
                    &mut data,
                    placement,
                    member.content.kind(),
                    start,
                    member.duration()?,
                    created,
                    next,
                )?;
                destinations.insert(*source, target);
                target
            }
        } else {
            *source
        };
        moves.push(Move {
            item: member.id,
            track: target,
            start,
        });
    }
    // The empty-main origin correction applies to the whole group, including audio, when no
    // member came from main: a group leaving main's origin keeps the gap it opens.
    let main = data.scene().timeline.main.id;
    let correction = if data.scene().timeline.main.items.is_empty()
        && !members.iter().any(|(source, _)| *source == main)
    {
        moves
            .iter()
            .filter(|m| m.track == main)
            .map(|m| m.start)
            .min()
            .filter(|start| *start > Tick::ZERO)
            .unwrap_or(Tick::ZERO)
    } else {
        Tick::ZERO
    };
    for movement in &mut moves {
        movement.start = sub(movement.start, correction)?;
    }
    // Restore source locations solely for the simultaneous move owner's lock checks.
    for (source, member) in members {
        track(&mut data, source)?.items.push(member);
    }
    move_items(&mut data, moves)?;
    Ok(data)
}

/// Delegate source-time, retime, keyframe and fade handling to the single-edge owner.
pub(super) fn trim_group(
    mut data: ProjectData,
    ids: Vec<ItemId>,
    edge: crate::TrimEdge,
    delta: Tick,
    sources: &mut TrimSources,
) -> Result<ProjectData> {
    let ids = with_group_members(&data, &ids);
    let mut seen = BTreeSet::new();
    if ids.is_empty() {
        return Err(EditError::Invalid("empty timeline group"));
    }
    for id in ids {
        if !seen.insert(id) {
            return Err(EditError::Invalid("duplicate group item"));
        }
        let member = item(&mut data, id)?;
        let at = super::add(
            match edge {
                crate::TrimEdge::Start => member.start,
                crate::TrimEdge::End => member.end()?,
            },
            delta,
        )?;
        data = match edge {
            crate::TrimEdge::Start => execute_trim_start(data, id, at, sources)?,
            crate::TrimEdge::End => execute_trim_end(data, id, at, sources)?,
        };
        let member = item(&mut data, id)?;
        let actual = match edge {
            crate::TrimEdge::Start => member.start,
            crate::TrimEdge::End => member.end()?,
        };
        if actual != at {
            return Err(EditError::Invalid(
                "group edge is not representable at playback rate",
            ));
        }
    }
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
    // Each piece of a compound clip owns a copy of the nested timeline under its own ids.
    compound::renumber(&mut right, next)?;
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
    let canvas = data.canvas.clone();
    let item = item(&mut data, id)?;
    let moved = item.start != at;
    let (animations, was) = (ClipAnimations::of(item, &canvas), item.duration()?);
    let cut = item.trim_start(at)?;
    let shift = sub(item.start, cut.start)?;
    set_range(item, cut.source_range, shift, sources)?;
    item.retime = cut.retime;
    item.start = cut.start;
    let clip = item.duration()?;
    animation::refit(item, animations, was, &canvas)?;
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
    let canvas = data.canvas.clone();
    let item = item(&mut data, id)?;
    let moved = item.end()? != at;
    let (animations, was) = (ClipAnimations::of(item, &canvas), item.duration()?);
    let cut = item.trim_end(at)?;
    set_range(item, cut.source_range, Tick::ZERO, sources)?;
    item.retime = cut.retime;
    let clip = item.duration()?;
    animation::refit(item, animations, was, &canvas)?;
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

/// Split every item at the playhead and keep only the right-hand piece.
pub(super) fn execute_split_left(
    mut data: ProjectData,
    items: Vec<ItemId>,
    at: Tick,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
    sources: &mut TrimSources,
) -> Result<ProjectData> {
    let mut left = Vec::new();
    for id in split_targets(&data, items)? {
        data = execute_split_at(data, id, at, created, next, sources)?;
        left.push(id);
    }
    execute_delete(data, left)
}

/// Split every item at the playhead and keep only the left-hand piece.
pub(super) fn execute_split_right(
    mut data: ProjectData,
    items: Vec<ItemId>,
    at: Tick,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
    sources: &mut TrimSources,
) -> Result<ProjectData> {
    let targets = split_targets(&data, items)?;
    let first_created = created.len();
    for id in targets {
        data = execute_split_at(data, id, at, created, next, sources)?;
    }
    let right: Vec<ItemId> = created
        .get(first_created..)
        .unwrap_or_default()
        .iter()
        .filter_map(|id| match id {
            CreatedId::Item(item) => Some(*item),
            _ => None,
        })
        .collect();
    // The kept left pieces are the originals; report only tracks created here.
    created.truncate(first_created);
    execute_delete(data, right)
}

/// Split every unlocked item that spans the playhead, on every track of the
/// active scene, as one step. Locked tracks are skipped; nothing to split refuses.
pub(super) fn execute_split_all(
    mut data: ProjectData,
    at: Tick,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
    sources: &mut TrimSources,
) -> Result<ProjectData> {
    let targets: Vec<ItemId> = data
        .scene()
        .timeline
        .iter()
        .filter(|track| !track.locked)
        .flat_map(|track| &track.items)
        .filter(|item| item.start < at && item.end().is_ok_and(|end| at < end))
        .map(|item| item.id)
        .collect();
    if targets.is_empty() {
        return Err(EditError::Invalid("nothing under playhead"));
    }
    for id in targets {
        data = execute_split_at(data, id, at, created, next, sources)?;
    }
    Ok(data)
}

/// Close the empty spans of one unlocked track. With `at`, only the gap that contains that
/// time closes; without it, every gap closes, the lead-in before the first item included.
/// Each item after a closed gap moves left by the gap's length, so the track stays one
/// contiguous run. Refuses when there is no gap to close; one undo step.
pub(super) fn execute_close_gaps(
    mut data: ProjectData,
    track_id: TrackId,
    at: Option<Tick>,
) -> Result<ProjectData> {
    let mut spans = Vec::new();
    for item in &track(&mut data, track_id)?.items {
        spans.push((item.start, item.end()?));
    }
    spans.sort_unstable();
    let mut gaps = Vec::new();
    let mut cursor = Tick::ZERO;
    for (start, end) in spans {
        if start > cursor {
            gaps.push((cursor, start));
        }
        cursor = cursor.max(end);
    }
    let closing = match at {
        Some(at) => {
            let gap = gaps
                .into_iter()
                .find(|(start, end)| *start <= at && at < *end)
                .ok_or(EditError::Invalid("no gap at time"))?;
            vec![gap]
        }
        None if gaps.is_empty() => return Err(EditError::Invalid("no gap to close")),
        None => gaps,
    };
    let cuts = crate::transitions::cuts(&data.scene().timeline);
    let target = track(&mut data, track_id)?;
    unlocked(target)?;
    for item in &mut target.items {
        let mut shift = Tick::ZERO;
        for (start, end) in &closing {
            if *end <= item.start {
                shift = super::add(shift, super::sub(*end, *start)?)?;
            }
        }
        if shift != Tick::ZERO {
            item.start = super::sub(item.start, shift)?;
        }
    }
    super::check_timeline(&mut data)?;
    crate::transitions::revalidate(&mut data, &cuts);
    Ok(data)
}

fn split_targets(data: &ProjectData, items: Vec<ItemId>) -> Result<Vec<ItemId>> {
    if items.is_empty() {
        return Err(EditError::Invalid("empty timeline group"));
    }
    let mut seen = BTreeSet::new();
    for id in &items {
        if !seen.insert(*id) {
            return Err(EditError::Invalid("duplicate split item"));
        }
        if find_item(data, *id).is_none() {
            return Err(EditError::NoItem(*id));
        }
    }
    Ok(items)
}

fn find_item(data: &ProjectData, id: ItemId) -> Option<(&Track, &Item)> {
    data.scene().timeline.iter().find_map(|track| {
        track
            .items
            .iter()
            .find(|item| item.id == id)
            .map(|item| (track, item))
    })
}

/// Clone every selected item onto one new track per source track, at the
/// highest position for that track's group, keeping the original start time.
pub(super) fn execute_duplicate(
    mut data: ProjectData,
    items: Vec<ItemId>,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
) -> Result<ProjectData> {
    if items.is_empty() {
        return Err(EditError::Invalid("empty timeline group"));
    }
    let items = with_group_members(&data, &items);
    let mut selected = BTreeSet::new();
    for id in items {
        if !selected.insert(id) {
            return Err(EditError::Invalid("duplicate item"));
        }
        let (source, _) = find_item(&data, id).ok_or(EditError::NoItem(id))?;
        unlocked(source)?;
    }
    let mut groups = Vec::new();
    for track in data.scene().timeline.iter() {
        let copies: Vec<Item> = track
            .items
            .iter()
            .filter(|item| selected.contains(&item.id))
            .cloned()
            .collect();
        if copies.is_empty() {
            continue;
        }
        groups.push((
            format!("{} (copy)", track.name.as_str()).into(),
            track.kind,
            copies,
        ));
    }
    let mut renamed = BTreeMap::new();
    for (name, kind, copies) in groups {
        let target =
            super::tracks::add_track(&mut data, name, kind, TrackPlacement::Auto, created, next)?;
        let target = track(&mut data, target)?;
        for mut copy in copies {
            copy.id = ItemId(alloc(next)?);
            if let Some(old) = copy.group {
                let new = match renamed.get(&old) {
                    Some(new) => *new,
                    None => {
                        let new = GroupId(alloc(next)?);
                        created.push(CreatedId::Group(new));
                        renamed.insert(old, new);
                        new
                    }
                };
                copy.group = Some(new);
            }
            compound::renumber(&mut copy, next)?;
            created.push(CreatedId::Item(copy.id));
            target.items.push(copy);
        }
    }

    Ok(data)
}

pub(super) fn execute_set_items_hidden(
    mut data: ProjectData,
    items: Vec<ItemId>,
    hidden: bool,
) -> Result<ProjectData> {
    let mut seen = BTreeSet::new();
    if items.is_empty() {
        return Err(EditError::Invalid("empty timeline group"));
    }
    for id in items {
        if !seen.insert(id) {
            return Err(EditError::Invalid("duplicate item"));
        }
        let item = item(&mut data, id)?;
        if item.content.kind() == TrackKind::Audio {
            return Err(EditError::Invalid("audio item has no video"));
        }
        item.hidden = hidden;
    }

    Ok(data)
}

pub(super) fn execute_set_items_muted(
    mut data: ProjectData,
    items: Vec<ItemId>,
    muted: bool,
) -> Result<ProjectData> {
    let mut seen = BTreeSet::new();
    if items.is_empty() {
        return Err(EditError::Invalid("empty timeline group"));
    }
    for id in items {
        if !seen.insert(id) {
            return Err(EditError::Invalid("duplicate item"));
        }
        match &mut item(&mut data, id)?.content {
            ItemParams::Video(params) => params.muted = muted,
            ItemParams::Audio(params) => params.muted = muted,
            _ => return Err(EditError::Invalid("item has no audio")),
        }
    }

    Ok(data)
}

/// Sets the clip background on video items; one undo step for the whole list.
/// A blur intensity above the accepted range is refused before any item changes.
pub(super) fn execute_set_items_background(
    mut data: ProjectData,
    items: Vec<ItemId>,
    background: project::ClipBackground,
) -> Result<ProjectData> {
    let mut seen = BTreeSet::new();
    if items.is_empty() {
        return Err(EditError::Invalid("empty timeline group"));
    }
    if !background.valid() {
        return Err(EditError::Invalid("clip background"));
    }
    for id in items {
        if !seen.insert(id) {
            return Err(EditError::Invalid("duplicate item"));
        }
        match &mut item(&mut data, id)?.content {
            ItemParams::Video(params) => params.background = background,
            _ => return Err(EditError::Invalid("item has no video frame")),
        }
    }

    Ok(data)
}

struct Extracted {
    video: ItemId,
    start: Tick,
    duration: Tick,
    source_range: SourceRange,
    retime: project::Retime,
    volume: f32,
    muted: bool,
    fades: project::Fades,
    volume_keys: Option<project::Param>,
}

/// Disable a video item's source audio and add an equivalent audio item on the
/// first free audio track (or a new one). The source is never decoded.
pub(super) fn execute_extract_audio(
    mut data: ProjectData,
    items: Vec<ItemId>,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
) -> Result<ProjectData> {
    if items.is_empty() {
        return Err(EditError::Invalid("empty timeline group"));
    }
    let mut seen = BTreeSet::new();
    let mut extracted = Vec::with_capacity(items.len());
    for id in items {
        if !seen.insert(id) {
            return Err(EditError::Invalid("duplicate item"));
        }
        let (source, item) = find_item(&data, id).ok_or(EditError::NoItem(id))?;
        unlocked(source)?;
        let ItemParams::Video(params) = &item.content else {
            return Err(EditError::Invalid("item is not video"));
        };
        if !params.source_audio_enabled {
            return Err(EditError::Invalid("source audio is extracted"));
        }
        let asset = item
            .source_asset
            .and_then(|id| data.assets.iter().find(|asset| asset.id() == id))
            .ok_or(EditError::Invalid("video has no source asset"))?;
        if !asset
            .probe()
            .streams
            .iter()
            .any(|stream| stream.audio.is_some())
        {
            return Err(EditError::Invalid("source has no audio"));
        }
        let path = project::AnimationPath::from(project::BuiltinPath::Volume);
        extracted.push(Extracted {
            video: id,
            start: item.start,
            duration: item.duration()?,
            source_range: item.source_range,
            retime: item.retime,
            volume: params.volume,
            muted: params.muted,
            fades: params.fades,
            volume_keys: item.animations.get(&path).cloned(),
        });
    }
    for source in extracted {
        if let ItemParams::Video(params) = &mut item(&mut data, source.video)?.content {
            params.source_audio_enabled = false;
        }
        let (target, _) = placement_track(
            &mut data,
            Placement::Auto,
            TrackKind::Audio,
            source.start,
            source.duration,
            created,
            next,
        )?;
        let id = ItemId(alloc(next)?);
        let mut animations = project::Animations::default();
        if let Some(param) = source.volume_keys {
            animations.set(
                project::AnimationPath::from(project::BuiltinPath::Volume),
                param,
            )?;
        }
        let source_asset = find_item(&data, source.video)
            .ok_or(EditError::NoItem(source.video))?
            .1
            .source_asset;
        track(&mut data, target)?.items.push(Item {
            hidden: false,
            blend_mode: Default::default(),
            group: None,
            masks: Vec::new(),
            effects: Vec::new(),
            animations,
            id,
            source_asset,
            source_range: source.source_range,
            start: source.start,
            retime: source.retime,
            content: ItemParams::Audio(project::AudioParams {
                volume: source.volume,
                muted: source.muted,
                fades: source.fades,
                ..project::AudioParams::default()
            }),
        });
        created.push(CreatedId::Item(id));
    }

    Ok(data)
}

/// Re-enable a video item's own source audio. The extracted audio item stays,
/// matching the reference toggle; it becomes an ordinary independent clip.
pub(super) fn execute_recover_audio(
    mut data: ProjectData,
    items: Vec<ItemId>,
) -> Result<ProjectData> {
    if items.is_empty() {
        return Err(EditError::Invalid("empty timeline group"));
    }
    let mut seen = BTreeSet::new();
    for id in items {
        if !seen.insert(id) {
            return Err(EditError::Invalid("duplicate item"));
        }
        match &mut item(&mut data, id)?.content {
            ItemParams::Video(params) if !params.source_audio_enabled => {
                params.source_audio_enabled = true;
            }
            ItemParams::Video(_) => return Err(EditError::Invalid("source audio is enabled")),
            _ => return Err(EditError::Invalid("item is not video")),
        }
    }

    Ok(data)
}
/// Ripple-delete an item with every member of its group, each closing its own span.
pub(super) fn execute_ripple_delete(mut data: ProjectData, id: ItemId) -> Result<ProjectData> {
    find_item(&data, id).ok_or(EditError::NoItem(id))?;
    let mut members: Vec<(Tick, ItemId)> = with_group_members(&data, &[id])
        .into_iter()
        .filter_map(|member| find_item(&data, member).map(|(_, item)| (item.start, member)))
        .collect();
    // Latest first, so each removal closes its own span before an earlier one shifts it.
    members.sort_by(|a, b| b.cmp(a));
    for (_, member) in members {
        data = ripple_delete_one(data, member)?;
    }
    Ok(data)
}

fn ripple_delete_one(mut data: ProjectData, id: ItemId) -> Result<ProjectData> {
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

/// Which measured words a reviewed span cut may remove.
pub(super) enum WordCut<'a> {
    /// No span may touch a measured word; every caption word stays intact.
    Clear,
    /// Exactly these words are removed; every word touching a span must be one of
    /// them and sit fully inside that span.
    Words(&'a BTreeSet<crate::WordId>),
}

/// Remove reviewed output ranges from every track. Items are split at the span
/// boundaries inside them, pieces fully inside a span are deleted, and the
/// session's ripple policy closes the vacated tails on each track. A nonzero
/// crossfade sets an equal-power fade pair on the surviving edges of each join.
pub(super) fn remove_output_ranges(
    mut data: ProjectData,
    spans: &[SourceRange],
    crossfade_ms: i64,
    cut: WordCut<'_>,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
    sources: &mut TrimSources,
) -> Result<ProjectData> {
    if spans.is_empty()
        || spans.len() > crate::session::wire::MAX_PAUSE_SPANS
        || !(0..=crate::session::wire::MAX_PAUSE_MS).contains(&crossfade_ms)
    {
        return Err(EditError::Invalid("reviewed spans"));
    }
    let crossfade = Tick::from_units(crossfade_ms, 1000, time::Rounding::Nearest)?;
    let mut timeline_end = Tick::ZERO;
    for track in data.scene().timeline.iter() {
        for item in &track.items {
            timeline_end = timeline_end.max(item.end()?);
        }
    }
    if timeline_end == Tick::ZERO {
        return Err(EditError::Invalid("reviewed cut needs a timeline"));
    }
    let mut previous = Tick::ZERO;
    for span in spans {
        if span.start() < previous || span.end() <= span.start() || span.end() > timeline_end {
            return Err(EditError::Invalid("reviewed span"));
        }
        previous = span.end();
    }
    let mut words = Vec::new();
    for track in data.scene().timeline.iter() {
        for item in &track.items {
            words.extend(item.word_spans()?);
        }
    }
    let mut removed_words = BTreeSet::new();
    for span in spans {
        for word in words
            .iter()
            .filter(|word| word.output.start() < span.end() && span.start() < word.output.end())
        {
            let id = crate::WordId {
                item: word.item,
                cue: word.cue.clone(),
                word: word.index,
            };
            let allowed = match &cut {
                WordCut::Clear => false,
                WordCut::Words(selected) => selected.contains(&id),
            };
            if !allowed || word.output.start() < span.start() || word.output.end() > span.end() {
                return Err(EditError::Invalid("reviewed cut crosses a word"));
            }
            removed_words.insert(id);
        }
    }
    if let WordCut::Words(selected) = cut
        && removed_words != *selected
    {
        return Err(EditError::Invalid("transcript word was not removed"));
    }

    let mut targets: Vec<(ItemId, Tick, Tick)> = Vec::new();
    for track in data.scene().timeline.iter() {
        for item in &track.items {
            let end = item.end()?;
            if spans
                .iter()
                .any(|span| item.start < span.end() && span.start() < end)
            {
                targets.push((item.id, item.start, end));
            }
        }
    }
    for (id, start, end) in targets {
        let mut bounds = Vec::new();
        for span in spans {
            for at in [span.start(), span.end()] {
                if at > start && at < end && !bounds.contains(&at) {
                    bounds.push(at);
                }
            }
        }
        // Descending keeps the original ID on the piece that still holds earlier bounds.
        bounds.sort_unstable_by(|a, b| b.cmp(a));
        for at in bounds {
            data = execute_split_at(data, id, at, created, next, sources)?;
        }
    }
    let mut removed = Vec::new();
    for track in data.scene().timeline.iter() {
        for item in &track.items {
            let end = item.end()?;
            if spans
                .iter()
                .any(|span| item.start >= span.start() && end <= span.end())
            {
                removed.push(item.id);
            }
        }
    }
    data = execute_delete(data, removed)?;

    if crossfade > Tick::ZERO {
        for span in spans {
            for track in data.scene_mut().timeline.iter_mut() {
                let mut left: Option<(Tick, ItemId)> = None;
                let mut right: Option<(Tick, ItemId)> = None;
                for item in &track.items {
                    let end = item.end()?;
                    if end == span.start() {
                        left = Some((item.start, item.id));
                    }
                    if item.start == span.end() {
                        right = Some((end, item.id));
                    }
                }
                if let Some((_, id)) = left
                    && let Some(item) = track.items.iter_mut().find(|item| item.id == id)
                {
                    let clip = item.duration()?;
                    if let Some(fades) = fades_mut(&mut item.content) {
                        fades.set(
                            project::FadeEdge::Out,
                            crossfade,
                            project::FadeCurve::EqualPower,
                            clip,
                        )?;
                    }
                }
                if let Some((_, id)) = right
                    && let Some(item) = track.items.iter_mut().find(|item| item.id == id)
                {
                    let clip = item.duration()?;
                    if let Some(fades) = fades_mut(&mut item.content) {
                        fades.set(
                            project::FadeEdge::In,
                            crossfade,
                            project::FadeCurve::EqualPower,
                            clip,
                        )?;
                    }
                }
            }
        }
    }

    Ok(data)
}

/// Tracks whose vacated tails the main-track magnet closes for one edit, or `None` when the
/// magnet does not apply. It applies when magnet is on, every leaf of the command is a delete
/// or split-and-remove of listed items, and a listed item sits on the main track. The scope is
/// every track holding a listed item, so a linked partner closes on its own track by the same
/// rule; any other track is never shifted.
pub(crate) fn magnet_scope(data: &ProjectData, commands: &[Command]) -> Option<BTreeSet<TrackId>> {
    if !data.scene().timeline.magnet {
        return None;
    }
    let mut targets = Vec::new();
    if !commands
        .iter()
        .all(|command| collect_removal_targets(command, &mut targets))
        || targets.is_empty()
    {
        return None;
    }
    // A group is one block on every track it touches, so its other members count as listed.
    let targets = with_group_members(data, &targets);
    let mut tracks = BTreeSet::new();
    for id in targets {
        let track = data
            .scene()
            .timeline
            .iter()
            .find(|track| track.items.iter().any(|item| item.id == id))?;
        tracks.insert(track.id);
    }
    tracks
        .contains(&data.scene().timeline.main.id)
        .then_some(tracks)
}

fn collect_removal_targets(command: &Command, targets: &mut Vec<ItemId>) -> bool {
    match command {
        Command::Delete { items }
        | Command::TimelineSplitLeft { items, .. }
        | Command::TimelineSplitRight { items, .. } => {
            targets.extend(items.iter().copied());
            true
        }
        Command::Batch { commands } => commands
            .iter()
            .all(|command| collect_removal_targets(command, targets)),
        _ => false,
    }
}

/// Close only same-track tails vacated by one complete edit candidate, on the tracks in
/// `scope` (`None` means every track).
pub(crate) fn close_vacated_tails(
    before: &ProjectData,
    after: project::Project,
    scope: Option<&BTreeSet<TrackId>>,
) -> Result<project::Project> {
    let locations: BTreeMap<_, _> = after
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| {
            track
                .items
                .iter()
                .map(move |item| (item.id, (track.id, item)))
        })
        .collect();
    let mut shifts = BTreeMap::new();
    for track in before.scene().timeline.iter() {
        if scope.is_some_and(|scope| !scope.contains(&track.id)) {
            continue;
        }
        let Some(next_track) = after
            .data()
            .scene()
            .timeline
            .iter()
            .find(|t| t.id == track.id)
        else {
            continue;
        };
        let mut vacated = Vec::new();
        for item in &track.items {
            let end = item.end()?;
            let start = match locations.get(&item.id) {
                // Moving to another track is a lift, never a deletion.
                Some((id, _)) if *id != track.id => continue,
                Some((_, next)) => next.end()?,
                None => item.start,
            };
            if start < end {
                vacated.push((start, end));
            }
        }
        // Group members can vacate overlapping tails when moved beyond their
        // former spans. Close their union once, preserving occupied material.
        vacated.sort_unstable();
        let mut merged: Vec<(Tick, Tick)> = Vec::new();
        for (start, end) in vacated {
            if let Some(last) = merged.last_mut()
                && start <= last.1
            {
                last.1 = last.1.max(end);
            } else {
                merged.push((start, end));
            }
        }
        let mut gaps = Vec::new();
        let mut occupied = next_track.items.iter().peekable();
        for (start, end) in merged {
            let mut cursor = start;
            // Subtract ALL occupied output, including split pieces and clips that
            // moved into a vacancy. A leftward move closes its entire tail.
            while let Some(next) = occupied.peek() {
                if next.start >= end {
                    break;
                }
                let next_end = next.end()?;
                if next_end <= cursor {
                    occupied.next();
                    continue;
                }
                if next.start > cursor {
                    gaps.push((cursor, next.start));
                }
                cursor = cursor.max(next_end);
                if cursor >= end {
                    break;
                }
                occupied.next();
            }
            if cursor < end {
                gaps.push((cursor, end));
            }
        }
        if !gaps.is_empty() {
            shifts.insert(track.id, gaps);
        }
    }
    if shifts.is_empty() {
        return Ok(after);
    }
    let cuts = crate::transitions::cuts(&after.data().scene().timeline);
    let mut data = after.into_data();
    for track in data.scene_mut().timeline.iter_mut() {
        let Some(gaps) = shifts.get(&track.id) else {
            continue;
        };
        let mut gaps = gaps.iter().peekable();
        let mut shift = Tick::ZERO;
        for item in &mut track.items {
            while let Some((start, end)) = gaps.peek() {
                if *end > item.start {
                    break;
                }
                shift = super::add(shift, super::sub(*end, *start)?)?;
                gaps.next();
            }
            if shift != Tick::ZERO {
                if track.locked {
                    return Err(EditError::Locked(track.id));
                }
                item.start = super::sub(item.start, shift)?;
            }
        }
    }
    super::check_timeline(&mut data)?;
    crate::transitions::revalidate(&mut data, &cuts);
    Ok(project::Project::new(data)?)
}

/// The listed items plus every other active-scene member of their groups, in track order.
pub(crate) fn with_group_members(data: &ProjectData, items: &[ItemId]) -> Vec<ItemId> {
    with_group_members_where(data, items, |_| true)
}

/// The listed items plus the group members that strictly contain the cut `at`, so a split
/// reaches every piece of the group under the playhead.
pub(super) fn with_group_members_at(data: &ProjectData, items: &[ItemId], at: Tick) -> Vec<ItemId> {
    with_group_members_where(data, items, |member| {
        member.start < at && member.end().is_ok_and(|end| at < end)
    })
}

fn with_group_members_where(
    data: &ProjectData,
    items: &[ItemId],
    keep: impl Fn(&Item) -> bool,
) -> Vec<ItemId> {
    let groups: BTreeSet<GroupId> = items
        .iter()
        .filter_map(|id| find_item(data, *id)?.1.group)
        .collect();
    let mut all = items.to_vec();
    if groups.is_empty() {
        return all;
    }
    let listed: BTreeSet<ItemId> = items.iter().copied().collect();
    for item in data.scene().timeline.iter().flat_map(|track| &track.items) {
        if item.group.is_some_and(|group| groups.contains(&group))
            && !listed.contains(&item.id)
            && keep(item)
        {
            all.push(item.id);
        }
    }
    all
}

/// Moves for the unlisted members of every group a move touches: each shifts by the delta of
/// the first listed member of its group and stays on its own track.
fn with_group_moves(data: &ProjectData, mut moves: Vec<Move>) -> Result<Vec<Move>> {
    let mut deltas = BTreeMap::new();
    for movement in &moves {
        if let Some((_, item)) = find_item(data, movement.item)
            && let Some(group) = item.group
            && !deltas.contains_key(&group)
        {
            deltas.insert(group, sub(movement.start, item.start)?);
        }
    }
    if deltas.is_empty() {
        return Ok(moves);
    }
    let listed: BTreeSet<ItemId> = moves.iter().map(|m| m.item).collect();
    for track in data.scene().timeline.iter() {
        for item in &track.items {
            if let Some(group) = item.group
                && let Some(delta) = deltas.get(&group)
                && !listed.contains(&item.id)
            {
                moves.push(Move {
                    item: item.id,
                    track: track.id,
                    start: super::add(item.start, *delta)?,
                });
            }
        }
    }
    Ok(moves)
}

/// Trim one edge to `at`; a grouped item trims every member of its group by the same delta.
pub(super) fn trim_set(
    data: ProjectData,
    id: ItemId,
    at: Tick,
    edge: crate::TrimEdge,
    sources: &mut TrimSources,
) -> Result<ProjectData> {
    let (_, item) = find_item(&data, id).ok_or(EditError::NoItem(id))?;
    if item.group.is_none() {
        return match edge {
            crate::TrimEdge::Start => execute_trim_start(data, id, at, sources),
            crate::TrimEdge::End => execute_trim_end(data, id, at, sources),
        };
    }
    let edge_now = match edge {
        crate::TrimEdge::Start => item.start,
        crate::TrimEdge::End => item.end()?,
    };
    let delta = sub(at, edge_now)?;
    let ids = with_group_members(&data, &[id]);
    trim_group(data, ids, edge, delta, sources)
}

/// Split the item and every member of its group that spans the cut; pieces keep the group.
pub(super) fn split_set(
    mut data: ProjectData,
    id: ItemId,
    at: Tick,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
    sources: &mut TrimSources,
) -> Result<ProjectData> {
    for target in with_group_members_at(&data, &[id], at) {
        data = execute_split_at(data, target, at, created, next, sources)?;
    }
    Ok(data)
}

/// Put the listed items, and every member of any group among them, into one new group.
pub(super) fn execute_group(
    mut data: ProjectData,
    items: Vec<ItemId>,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
) -> Result<ProjectData> {
    let mut seen = BTreeSet::new();
    for id in &items {
        if !seen.insert(*id) {
            return Err(EditError::Invalid("duplicate group item"));
        }
        find_item(&data, *id).ok_or(EditError::NoItem(*id))?;
    }
    let members: BTreeSet<ItemId> = with_group_members(&data, &items).into_iter().collect();
    if members.len() < 2 {
        return Err(EditError::GroupTooSmall);
    }
    let existing: BTreeSet<Option<GroupId>> = members
        .iter()
        .filter_map(|id| find_item(&data, *id))
        .map(|(_, item)| item.group)
        .collect();
    if existing.len() == 1 && existing.iter().all(Option::is_some) {
        return Err(EditError::AlreadyGrouped);
    }
    for id in &members {
        unlocked(find_item(&data, *id).ok_or(EditError::NoItem(*id))?.0)?;
    }
    let group = GroupId(alloc(next)?);
    created.push(CreatedId::Group(group));
    for track in data.scene_mut().timeline.iter_mut() {
        for item in &mut track.items {
            if members.contains(&item.id) {
                item.group = Some(group);
            }
        }
    }
    Ok(data)
}

/// Dissolve the whole group of every listed item; a listed item outside any group refuses.
pub(super) fn execute_ungroup(mut data: ProjectData, items: Vec<ItemId>) -> Result<ProjectData> {
    if items.is_empty() {
        return Err(EditError::Invalid("empty timeline group"));
    }
    let mut groups = BTreeSet::new();
    for id in &items {
        let (_, item) = find_item(&data, *id).ok_or(EditError::NoItem(*id))?;
        groups.insert(item.group.ok_or(EditError::NotGrouped(*id))?);
    }
    for track in data.scene().timeline.iter() {
        let touched = track
            .items
            .iter()
            .any(|item| item.group.is_some_and(|group| groups.contains(&group)));
        if touched {
            unlocked(track)?;
        }
    }
    for track in data.scene_mut().timeline.iter_mut() {
        for item in &mut track.items {
            if item.group.is_some_and(|group| groups.contains(&group)) {
                item.group = None;
            }
        }
    }
    Ok(data)
}

/// Dissolve every group left with fewer than two members, in any scene.
pub(super) fn dissolve_singleton_groups(data: &mut ProjectData) {
    let mut counts: BTreeMap<GroupId, usize> = BTreeMap::new();
    for scene in data.scenes.iter() {
        for group in scene
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .filter_map(|item| item.group)
        {
            *counts.entry(group).or_default() += 1;
        }
    }
    if counts.values().all(|count| *count >= 2) {
        return;
    }
    for scene in data.scenes.iter_mut() {
        for track in scene.timeline.iter_mut() {
            for item in &mut track.items {
                if item
                    .group
                    .is_some_and(|group| counts.get(&group).is_some_and(|n| *n < 2))
                {
                    item.group = None;
                }
            }
        }
    }
}
