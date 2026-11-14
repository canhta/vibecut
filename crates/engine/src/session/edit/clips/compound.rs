//! Moves timeline items into compound clips, or compound clips back into their members.
use super::super::{
    BTreeMap, BTreeSet, CreatedId, EditError, Item, ItemId, ItemParams, ProjectData, Result, Tick,
    Track, TrackId, TrackKind, alloc, unlocked,
};

/// `timeline.compound`: move `items` into one new compound clip at their earliest start.
///
/// The members keep their ids and their relative times on nested tracks in their outer order.
/// The clip goes onto the lowest video track that held a member when the clip's span is free
/// there, else onto a new video track directly above the lowest visual track that held one.
/// A transition moves in with its clips; one joining a member to a non-member refuses.
pub(in crate::session::edit) fn execute_compound(
    mut data: ProjectData,
    items: Vec<ItemId>,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
) -> Result<ProjectData> {
    if items.is_empty() {
        return Err(EditError::Invalid("empty timeline group"));
    }
    let mut selected = BTreeSet::new();
    for id in &items {
        if !selected.insert(*id) {
            return Err(EditError::Invalid("duplicate item"));
        }
    }
    let timeline = &data.scene().timeline;
    let mut start = None::<Tick>;
    let mut end = Tick::ZERO;
    for id in &selected {
        let (track, item) = timeline
            .iter()
            .find_map(|track| {
                track
                    .items
                    .iter()
                    .find(|item| item.id == *id)
                    .map(|item| (track, item))
            })
            .ok_or(EditError::NoItem(*id))?;
        unlocked(track)?;
        start = Some(start.map_or(item.start, |start| start.min(item.start)));
        end = end.max(item.end()?);
    }
    let start = start.ok_or(EditError::Invalid("empty timeline group"))?;
    let mut inside = BTreeSet::new();
    for transition in &timeline.transitions {
        let joined: Vec<ItemId> = [transition.from, transition.to]
            .into_iter()
            .chain(transition.from_pieces.iter().copied())
            .chain(transition.to_pieces.iter().copied())
            .collect();
        let members = joined.iter().filter(|id| selected.contains(id)).count();
        if members == joined.len() {
            inside.insert(transition.id);
        } else if members > 0 {
            return Err(EditError::CompoundSplitsTransition(transition.id));
        }
    }
    // Bottom to top, the outer tracks that hold a member.
    let order: Vec<(TrackId, TrackKind)> = timeline
        .iter()
        .filter(|track| track.items.iter().any(|item| selected.contains(&item.id)))
        .map(|track| (track.id, track.kind))
        .collect();
    // The clip's id comes first, so a fixed project always names the same clip.
    let id = ItemId(alloc(next)?);
    let timeline = &mut data.scene_mut().timeline;
    let mut renamed = BTreeMap::new();
    let main_items = take_members(&mut timeline.main, &selected, start)?;
    let main = nested_track(&timeline.main, TrackId(alloc(next)?), main_items);
    renamed.insert(timeline.main.id, main.id);
    let mut overlays = Vec::new();
    for track in &mut timeline.overlays {
        let members = take_members(track, &selected, start)?;
        if !members.is_empty() {
            let nested = nested_track(track, TrackId(alloc(next)?), members);
            renamed.insert(track.id, nested.id);
            overlays.push(nested);
        }
    }
    let mut audio = Vec::new();
    for track in &mut timeline.audio {
        let members = take_members(track, &selected, start)?;
        if !members.is_empty() {
            let nested = nested_track(track, TrackId(alloc(next)?), members);
            renamed.insert(track.id, nested.id);
            audio.push(nested);
        }
    }
    let (mut transitions, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut timeline.transitions)
        .into_iter()
        .partition(|transition| inside.contains(&transition.id));
    timeline.transitions = kept;
    for transition in &mut transitions {
        transition.track = *renamed
            .get(&transition.track)
            .ok_or(EditError::Invalid("compound transition track"))?;
        transition.start = transition
            .start
            .checked_sub(start)
            .ok_or(EditError::Invalid("compound transition time"))?;
    }
    let nested = project::Timeline {
        transitions,
        overlays,
        main,
        magnet: false,
        audio,
    };
    let compound = Item::compound(id, start, nested)?;
    let free = |track: &Track| {
        track
            .items
            .iter()
            .all(|item| item.start >= end || item.end().is_ok_and(|item_end| item_end <= start))
    };
    let host = order
        .iter()
        .find(|(_, kind)| *kind == TrackKind::Video)
        .map(|(track, _)| *track)
        .filter(|host| {
            timeline
                .iter()
                .any(|track| track.id == *host && free(track))
        });
    let host = match host {
        Some(host) => host,
        None => {
            let lowest = order
                .iter()
                .find(|(_, kind)| *kind != TrackKind::Audio)
                .map(|(track, _)| *track);
            // `overlays` is topmost first: inserting at a track's index puts the new one above it.
            let at = match lowest {
                Some(track) if track != timeline.main.id => timeline
                    .overlays
                    .iter()
                    .position(|overlay| overlay.id == track)
                    .ok_or(EditError::NoTrack(track))?,
                _ => timeline.overlays.len(),
            };
            let track = TrackId(alloc(next)?);
            timeline.overlays.insert(
                at,
                Track {
                    id: track,
                    name: format!("{:?}", TrackKind::Video).into(),
                    kind: TrackKind::Video,
                    muted: false,
                    hidden: false,
                    locked: false,
                    items: Vec::new(),
                },
            );
            created.push(CreatedId::Track(track));
            track
        }
    };
    let host = timeline
        .iter_mut()
        .find(|track| track.id == host)
        .ok_or(EditError::NoTrack(host))?;
    let index = host.items.partition_point(|item| item.start <= start);
    host.items.insert(index, compound);
    created.push(CreatedId::Item(id));
    Ok(data)
}

/// The selected items of `track`, removed and moved to compound-local time.
fn take_members(track: &mut Track, selected: &BTreeSet<ItemId>, start: Tick) -> Result<Vec<Item>> {
    let (mut members, kept): (Vec<Item>, Vec<Item>) = std::mem::take(&mut track.items)
        .into_iter()
        .partition(|item| selected.contains(&item.id));
    track.items = kept;
    for member in &mut members {
        member.start = member
            .start
            .checked_sub(start)
            .ok_or(EditError::Invalid("compound member time"))?;
        // The compound clip stands for its group inside the scene; members carry none.
        member.group = None;
    }
    Ok(members)
}

/// A nested track that keeps the outer track's name, kind and hidden and muted state.
fn nested_track(outer: &Track, id: TrackId, items: Vec<Item>) -> Track {
    Track {
        id,
        name: outer.name.clone(),
        kind: outer.kind,
        muted: outer.muted,
        hidden: outer.hidden,
        locked: false,
        items,
    }
}

/// `timeline.uncompound`: put each listed compound clip back as its members
/// ([`project::release_compound`]).
pub(in crate::session::edit) fn execute_uncompound(
    mut data: ProjectData,
    items: Vec<ItemId>,
) -> Result<ProjectData> {
    if items.is_empty() {
        return Err(EditError::Invalid("empty timeline group"));
    }
    let mut seen = BTreeSet::new();
    for id in &items {
        if !seen.insert(*id) {
            return Err(EditError::Invalid("duplicate item"));
        }
        let (track, item) = data
            .scene()
            .timeline
            .iter()
            .find_map(|track| {
                track
                    .items
                    .iter()
                    .find(|item| item.id == *id)
                    .map(|item| (track, item))
            })
            .ok_or(EditError::NoItem(*id))?;
        unlocked(track)?;
        if !matches!(item.content, ItemParams::Compound(_)) {
            return Err(EditError::NotCompound(*id));
        }
        // Members keep their own timing, so the clip's own trim, speed, look and sound
        // would be lost: refuse instead of baking them in.
        if item.compound_has_own_edits()? {
            return Err(EditError::CompoundHasOwnEdits(*id));
        }
    }
    for id in items {
        project::release_compound(&mut data.scene_mut().timeline, id)?;
    }
    Ok(data)
}

/// Fresh ids for everything a copied or split compound clip nests; other items are unchanged.
pub(in crate::session::edit) fn renumber(item: &mut Item, next: &mut Option<u64>) -> Result<()> {
    if let ItemParams::Compound(params) = &mut item.content {
        params.renumber(&mut || alloc(next))?;
    }
    Ok(())
}

/// Compound content enters a project only through `timeline.compound`, which allocates its ids.
pub(in crate::session::edit) fn admit_content(content: &ItemParams) -> Result<()> {
    if matches!(content, ItemParams::Compound(_)) {
        return Err(EditError::Unsupported(
            "compound clips are built by timeline.compound",
        ));
    }
    Ok(())
}
