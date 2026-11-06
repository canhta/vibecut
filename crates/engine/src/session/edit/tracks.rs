//! Applies tracks commands to a candidate project.
use super::{
    CreatedId, EditError, Placement, ProjectData, Result, Tick, Track, TrackId, TrackKind,
    TrackPlacement, add, alloc, track, unlocked,
};

pub(super) fn placement_track(
    data: &mut ProjectData,
    placement: Placement,
    kind: TrackKind,
    start: Tick,
    duration: Tick,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
) -> Result<(TrackId, bool)> {
    let secondary = match placement {
        Placement::Track { track } => return Ok((track, false)),
        Placement::Above { track } => TrackPlacement::Above { track },
        Placement::Below { track } => TrackPlacement::Below { track },
        Placement::Auto => {
            let main = &data.scene().timeline.main;
            if kind == TrackKind::Video
                && !main.locked
                && (data.scene().timeline.magnet || main.items.is_empty())
            {
                return Ok((main.id, data.scene().timeline.magnet));
            }
            let end = add(start, duration)?;
            // Main first, then overlays top-to-bottom, then audio top-to-bottom.
            if let Some(track) = std::iter::once(main)
                .chain(&data.scene().timeline.overlays)
                .chain(&data.scene().timeline.audio)
                .find(|t| {
                    t.kind == kind
                        && !t.locked
                        && t.items
                            .iter()
                            .all(|item| item.end().is_ok_and(|e| e <= start || item.start >= end))
                })
            {
                return Ok((track.id, false));
            }
            TrackPlacement::Auto
        }
    };
    let id = add_track(
        data,
        format!("{kind:?}").into(),
        kind,
        secondary,
        created,
        next,
    )?;
    Ok((id, false))
}

/// Resolve a screen-order insertion without crossing the main/audio boundary.
pub(super) fn add_track(
    data: &mut ProjectData,
    name: captions::NfcText,
    kind: TrackKind,
    placement: TrackPlacement,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
) -> Result<TrackId> {
    let audio = kind == TrackKind::Audio;
    let group = if audio {
        &data.scene().timeline.audio
    } else {
        &data.scene().timeline.overlays
    };
    let index = match placement {
        TrackPlacement::Auto => 0,
        TrackPlacement::Above { track } | TrackPlacement::Below { track } => {
            let below = matches!(placement, TrackPlacement::Below { .. });
            if track == data.scene().timeline.main.id {
                match (audio, below) {
                    (false, false) => group.len(),
                    (true, true) => 0,
                    _ => return Err(EditError::Invalid("track group boundary")),
                }
            } else {
                let reference = data
                    .scene()
                    .timeline
                    .iter()
                    .find(|t| t.id == track)
                    .ok_or(EditError::NoTrack(track))?;
                if (reference.kind == TrackKind::Audio) != audio {
                    return Err(EditError::Invalid("track group boundary"));
                }
                group
                    .iter()
                    .position(|t| t.id == track)
                    .ok_or(EditError::NoTrack(track))?
                    + usize::from(below)
            }
        }
    };
    let id = TrackId(alloc(next)?);
    let track = Track {
        id,
        name,
        kind,
        muted: false,
        hidden: false,
        locked: false,
        items: Vec::new(),
    };
    if matches!(placement, TrackPlacement::Auto) {
        data.scene_mut().timeline.insert_track(track);
    } else {
        let group = if audio {
            &mut data.scene_mut().timeline.audio
        } else {
            &mut data.scene_mut().timeline.overlays
        };
        group.insert(index, track);
    }
    created.push(CreatedId::Track(id));
    Ok(id)
}
/// Move a secondary track to a screen-order index inside its own section.
/// `index` counts from the top of the overlay or audio section; the mandatory
/// main track cannot move. Track names, order and lock state are project data.
pub(super) fn reorder_track(data: &mut ProjectData, id: TrackId, index: u32) -> Result<()> {
    let timeline = &mut data.scene_mut().timeline;
    if timeline.main.id == id {
        return Err(EditError::Invalid("main track cannot be moved"));
    }
    let index = index as usize;
    if let Some(position) = timeline.overlays.iter().position(|track| track.id == id) {
        if index >= timeline.overlays.len() {
            return Err(EditError::Invalid("track index"));
        }
        let track = timeline.overlays.remove(position);
        timeline.overlays.insert(index, track);
        return Ok(());
    }
    if let Some(position) = timeline.audio.iter().position(|track| track.id == id) {
        if index >= timeline.audio.len() {
            return Err(EditError::Invalid("track index"));
        }
        let track = timeline.audio.remove(position);
        timeline.audio.insert(index, track);
        return Ok(());
    }
    Err(EditError::NoTrack(id))
}
pub(super) fn execute_add_track(
    mut data: ProjectData,
    name: captions::NfcText,
    kind: TrackKind,
    placement: TrackPlacement,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
) -> Result<ProjectData> {
    add_track(&mut data, name, kind, placement, created, next)?;

    Ok(data)
}
pub(super) fn execute_remove_track(mut data: ProjectData, id: TrackId) -> Result<ProjectData> {
    if data.scene_mut().timeline.main.id == id {
        return Err(EditError::Invalid("main track cannot be removed"));
    }
    unlocked(track(&mut data, id)?)?;
    data.scene_mut().timeline.overlays.retain(|t| t.id != id);
    data.scene_mut().timeline.audio.retain(|t| t.id != id);

    Ok(data)
}
pub(super) fn execute_set_track_muted(
    mut data: ProjectData,
    id: TrackId,
    muted: bool,
) -> Result<ProjectData> {
    let t = track(&mut data, id)?;
    if !t.kind.can_mute() {
        return Err(EditError::Invalid("track has no audio"));
    }
    t.muted = muted;

    Ok(data)
}
pub(super) fn execute_set_track_hidden(
    mut data: ProjectData,
    id: TrackId,
    hidden: bool,
) -> Result<ProjectData> {
    let t = track(&mut data, id)?;
    if !t.kind.can_hide() {
        return Err(EditError::Invalid("audio track has no video"));
    }
    t.hidden = hidden;

    Ok(data)
}
pub(super) fn execute_set_track_magnet(
    mut data: ProjectData,
    enabled: bool,
) -> Result<ProjectData> {
    data.scene_mut().timeline.magnet = enabled;
    Ok(data)
}
pub(super) fn execute_set_track_flags(
    mut data: ProjectData,
    id: TrackId,
    muted: bool,
    hidden: bool,
    locked: bool,
) -> Result<ProjectData> {
    let t = track(&mut data, id)?;
    t.muted = muted;
    t.hidden = hidden;
    t.locked = locked;

    Ok(data)
}
pub(super) fn execute_set_track_locked(
    mut data: ProjectData,
    id: TrackId,
    locked: bool,
) -> Result<ProjectData> {
    track(&mut data, id)?.locked = locked;

    Ok(data)
}
pub(super) fn execute_rename_track(
    mut data: ProjectData,
    id: TrackId,
    name: captions::NfcText,
) -> Result<ProjectData> {
    // Renaming is project metadata, not an item structural edit; a
    // locked track can still be labelled (and unlocked) here.
    track(&mut data, id)?.name = name;

    Ok(data)
}
pub(super) fn execute_reorder_track(
    mut data: ProjectData,
    id: TrackId,
    index: u32,
) -> Result<ProjectData> {
    reorder_track(&mut data, id, index)?;

    Ok(data)
}
