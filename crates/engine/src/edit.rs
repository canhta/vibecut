use crate::{Command, CreatedId, EditError, Move, Result};
use project::{Bookmark, BookmarkId, Item, ItemId, ItemParams, ProjectData, Track, TrackId};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use time::{SourceRange, Tick};

pub(crate) type TrimSources = BTreeMap<ItemId, Arc<captions::CueTrack>>;

pub(crate) fn check_budget(command: &Command) -> Result<()> {
    let mut stack = vec![(command, 0)];
    let mut count = 0;
    while let Some((command, depth)) = stack.pop() {
        count += 1;
        if count > 1024 || depth > 16 {
            return Err(EditError::Limit);
        }
        match command {
            Command::Batch { commands } => {
                if commands.len() > 1024 {
                    return Err(EditError::Limit);
                }
                stack.extend(commands.iter().map(|c| (c, depth + 1)));
            }
            Command::MoveItems { moves } if moves.len() > 100_000 => return Err(EditError::Limit),
            Command::Delete { items } if items.len() > 100_000 => return Err(EditError::Limit),
            Command::Undo {}
            | Command::Redo {}
            | Command::BeginGesture {}
            | Command::CommitGesture {}
            | Command::CancelGesture {} => {
                return Err(EditError::Invalid("session control inside batch"));
            }
            _ => {}
        }
    }
    Ok(())
}
fn alloc(next: &mut Option<u64>) -> Result<u64> {
    let id = next.ok_or(EditError::IdExhausted)?;
    *next = id.checked_add(1);
    Ok(id)
}
fn track(data: &mut ProjectData, id: TrackId) -> Result<&mut Track> {
    data.timeline
        .tracks
        .iter_mut()
        .find(|t| t.id == id)
        .ok_or(EditError::NoTrack(id))
}
fn unlocked(track: &Track) -> Result<()> {
    if track.locked {
        Err(EditError::Locked(track.id))
    } else {
        Ok(())
    }
}
fn item(data: &mut ProjectData, id: ItemId) -> Result<&mut Item> {
    for t in &mut data.timeline.tracks {
        if t.items.iter().any(|i| i.id == id) {
            unlocked(t)?;
            return t
                .items
                .iter_mut()
                .find(|i| i.id == id)
                .ok_or(EditError::NoItem(id));
        }
    }
    Err(EditError::NoItem(id))
}
fn take_item(data: &mut ProjectData, id: ItemId) -> Result<Item> {
    for t in &mut data.timeline.tracks {
        if let Some(index) = t.items.iter().position(|i| i.id == id) {
            unlocked(t)?;
            return Ok(t.items.remove(index));
        }
    }
    Err(EditError::NoItem(id))
}
fn add(a: Tick, b: Tick) -> Result<Tick> {
    a.checked_add(b).ok_or(EditError::Invalid("time overflow"))
}
fn sub(a: Tick, b: Tick) -> Result<Tick> {
    a.checked_sub(b).ok_or(EditError::Invalid("time overflow"))
}

/// Keep caption clipping with its mapping owner, retaining source cue identity and text.
/// The caller supplies authoritative cues, never a previously clipped preview.
fn set_range(item: &mut Item, range: SourceRange, sources: &mut TrimSources) -> Result<()> {
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
    let shift = sub(item.source_range.start(), range.start())?;
    item.animations.shift(shift)?;
    item.source_range = range;
    Ok(())
}
fn move_items(data: &mut ProjectData, moves: Vec<Move>) -> Result<()> {
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
    for (id, item) in items {
        let dest = track(data, id)?;
        unlocked(dest)?;
        dest.items.push(item);
    }
    Ok(())
}
pub(crate) fn apply(
    mut data: ProjectData,
    command: Command,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
    sources: &mut TrimSources,
) -> Result<project::Project> {
    match command {
        Command::AddAsset { asset } => data.assets.push(asset),
        Command::AddTrack { name, kind } => {
            let id = TrackId(alloc(next)?);
            data.timeline.tracks.push(Track {
                id,
                name,
                kind,
                muted: false,
                hidden: false,
                locked: false,
                items: Vec::new(),
            });
            created.push(CreatedId::Track(id));
        }
        Command::RemoveTrack { track: id } => {
            unlocked(track(&mut data, id)?)?;
            data.timeline.tracks.retain(|t| t.id != id);
        }
        Command::SetTrackFlags {
            track: id,
            muted,
            hidden,
            locked,
        } => {
            let t = track(&mut data, id)?;
            t.muted = muted;
            t.hidden = hidden;
            t.locked = locked;
        }
        Command::InsertItem {
            track: id,
            item: new,
        } => {
            let t = track(&mut data, id)?;
            unlocked(t)?;
            let id = ItemId(alloc(next)?);
            t.items.push(Item {
                animations: new.animations,
                id,
                source_asset: new.source_asset,
                source_range: new.source_range,
                start: new.start,
                content: new.content,
            });
            created.push(CreatedId::Item(id));
        }
        Command::MoveItem { item, track, start } => {
            move_items(&mut data, vec![Move { item, track, start }])?
        }
        Command::MoveItems { moves } => move_items(&mut data, moves)?,
        Command::SplitAt { item: id, at } => {
            let left = item(&mut data, id)?;
            if at <= left.start || at >= left.end()? {
                return Err(EditError::Invalid("split must be inside item"));
            }
            let cut = add(left.source_range.start(), sub(at, left.start)?)?;
            let mut right = left.clone();
            let end = left.source_range.end();
            set_range(
                left,
                SourceRange::new(left.source_range.start(), cut)?,
                sources,
            )?;
            set_range(&mut right, SourceRange::new(cut, end)?, sources)?;
            // A split establishes independent retained content for both new pieces.
            if let ItemParams::Caption(p) = &left.content {
                sources.insert(left.id, Arc::new(p.cues.clone()));
            }
            right.id = ItemId(alloc(next)?);
            right.start = at;
            if let ItemParams::Caption(p) = &right.content {
                sources.insert(right.id, Arc::new(p.cues.clone()));
            }
            created.push(CreatedId::Item(right.id));
            let t = data
                .timeline
                .tracks
                .iter_mut()
                .find(|t| t.items.iter().any(|i| i.id == id))
                .ok_or(EditError::NoItem(id))?;
            t.items.push(right);
        }
        Command::TrimStart { item: id, at } => {
            let item = item(&mut data, id)?;
            let source = add(item.source_range.start(), sub(at, item.start)?)?;
            let range = SourceRange::new(source, item.source_range.end())?;
            set_range(item, range, sources)?;
            item.start = at;
        }
        Command::TrimEnd { item: id, at } => {
            let item = item(&mut data, id)?;
            let end = add(item.source_range.start(), sub(at, item.start)?)?;
            set_range(
                item,
                SourceRange::new(item.source_range.start(), end)?,
                sources,
            )?;
        }
        Command::Delete { items } => {
            let mut ids = BTreeSet::new();
            for id in items {
                if !ids.insert(id) {
                    return Err(EditError::Invalid("duplicate delete item"));
                }
                take_item(&mut data, id)?;
            }
        }
        Command::RippleDelete { item: id } => {
            let t = data
                .timeline
                .tracks
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
                    item.start = sub(item.start, removed.source_range.duration())?;
                }
            }
        }
        Command::SetItemParam { item: id, content } => {
            item(&mut data, id)?.content = content;
            // Explicit content replacement becomes authoritative for later previews.
            sources.remove(&id);
        }
        Command::SetAnimatedParam {
            item: id,
            path,
            param,
        } => {
            item(&mut data, id)?.animations.set(path, param)?;
        }
        Command::AddKeyframe {
            item: id,
            path,
            component,
            key,
        } => {
            item(&mut data, id)?
                .animations
                .get_mut(&path)?
                .channel_mut(component)?
                .put(key)?;
        }
        Command::RemoveKeyframe {
            item: id,
            path,
            component,
            time,
        } => {
            item(&mut data, id)?
                .animations
                .get_mut(&path)?
                .channel_mut(component)?
                .remove(time)?;
        }
        Command::RetimeKeyframe {
            item: id,
            path,
            component,
            from,
            to,
        } => {
            item(&mut data, id)?
                .animations
                .get_mut(&path)?
                .channel_mut(component)?
                .retime_key(from, to)?;
        }
        Command::SetKeyframeHandle {
            item: id,
            path,
            component,
            time,
            side,
            handle,
            tangent,
        } => {
            item(&mut data, id)?
                .animations
                .get_mut(&path)?
                .channel_mut(component)?
                .set_handle(time, side, handle, tangent)?;
        }
        Command::ScaleKeyframeTime {
            item: id,
            numerator,
            denominator,
        } => {
            item(&mut data, id)?
                .animations
                .retime(numerator, denominator)?;
        }
        Command::AddBookmark {
            start,
            duration,
            note,
            color,
        } => {
            let id = BookmarkId(alloc(next)?);
            data.bookmarks.push(Bookmark {
                id,
                start,
                duration,
                note,
                color,
            });
            created.push(CreatedId::Bookmark(id));
        }
        Command::Retranslate {
            item: id,
            target_language,
            texts,
            translation_ref,
            force,
        } => {
            if texts.is_empty()
                || texts.len() > ai::MAX_SEGMENTS
                || translation_ref.as_str().trim().is_empty()
                || translation_ref.as_str().len() > 256
            {
                return Err(EditError::Invalid("translation application"));
            }
            let item = item(&mut data, id)?;
            let ItemParams::Caption(params) = &mut item.content else {
                return Err(EditError::Invalid("translation requires caption item"));
            };
            if texts
                .keys()
                .any(|id| !params.cues.cues.iter().any(|c| &c.id == id))
                || texts.values().any(|t| t.as_str().trim().is_empty())
            {
                return Err(EditError::Invalid("translation cue ids or text"));
            }
            // A selected text edit does not replace the full caption content.
            // Update the retained track, then let the mapping owner rebuild the
            // clipped view so reversal can restore hidden cues and original ranges.
            let retained = sources.contains_key(&id);
            let cues = match sources.get_mut(&id) {
                Some(source) => Arc::make_mut(source),
                None => &mut params.cues,
            };
            if texts
                .keys()
                .any(|id| !cues.cues.iter().any(|cue| &cue.id == id))
            {
                return Err(EditError::Invalid("missing retained translation cue"));
            }
            for cue in &mut cues.cues {
                if let Some(text) = texts.get(&cue.id) {
                    if cue.source_lang == target_language {
                        return Err(EditError::Invalid("cannot replace source with translation"));
                    }
                    cue.set_text(
                        target_language.clone(),
                        captions::CueText {
                            text: text.clone(),
                            edited: false,
                            translation_ref: Some(translation_ref.clone()),
                        },
                        force,
                    );
                }
            }
            if retained {
                set_range(item, item.source_range, sources)?;
            }
        }
        Command::Batch { commands } => {
            for command in commands {
                // Consume each candidate before applying the next child; validation must
                // not clone an unvalidated allocation or publish a partial batch.
                data = apply(data, command, created, next, sources)?.into_data();
            }
        }
        Command::Undo {}
        | Command::Redo {}
        | Command::BeginGesture {}
        | Command::CommitGesture {}
        | Command::CancelGesture {} => {
            return Err(EditError::Invalid("session control inside batch"));
        }
    }
    check_timeline(&mut data)?;
    Ok(project::Project::new(data)?)
}
fn check_timeline(data: &mut ProjectData) -> Result<()> {
    for track in &mut data.timeline.tracks {
        track.items.sort_by_key(|i| i.start);
        let mut end = Tick::ZERO;
        for item in &track.items {
            if item.start < Tick::ZERO {
                return Err(EditError::Invalid("negative output start"));
            }
            if item.start < end {
                return Err(EditError::Overlap(track.id));
            }
            end = item.end()?;
        }
    }
    Ok(())
}
