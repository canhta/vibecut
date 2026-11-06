//! Applies transitions commands to a candidate project.
use super::{CreatedId, EditError, ItemId, ProjectData, Result, Tick, TrackId, alloc};
#[allow(clippy::too_many_arguments)]
pub(super) fn execute_add_transition(
    mut data: ProjectData,
    track: TrackId,
    from: ItemId,
    to: ItemId,
    kind: String,
    duration: Tick,
    alignment: project::TransitionAlignment,
    created: &mut Vec<CreatedId>,
    next: &mut Option<u64>,
) -> Result<ProjectData> {
    project::transition_definition(&kind)?;
    if alignment == project::TransitionAlignment::Custom {
        return Err(EditError::Invalid("custom alignment requires move"));
    }
    let mut tr = project::Transition {
        from_pieces: vec![],
        to_pieces: vec![],
        id: project::TransitionId(alloc(next)?),
        track,
        from,
        to,
        kind,
        start: Tick::ZERO,
        duration: Tick::ZERO,
        alignment,
    };
    crate::transitions::unlocked(&data.scene().timeline, &tr)?;
    let bounds = tr.bounds(&data.scene().timeline, &data)?;
    tr.set_duration(duration, bounds)?;
    created.push(CreatedId::Transition(tr.id));
    data.scene_mut().timeline.transitions.push(tr);

    Ok(data)
}
pub(super) fn execute_move_transition(
    mut data: ProjectData,
    transition: project::TransitionId,
    start: Tick,
) -> Result<ProjectData> {
    let n = crate::transitions::find(&data, transition)?;
    let mut tr = data
        .scene()
        .timeline
        .transitions
        .get(n)
        .cloned()
        .ok_or(EditError::Invalid("missing transition"))?;
    let bounds = tr.bounds(&data.scene().timeline, &data)?;
    crate::transitions::move_span(&mut tr, start, bounds)?;
    *data
        .scene_mut()
        .timeline
        .transitions
        .get_mut(n)
        .ok_or(EditError::Invalid("missing transition"))? = tr;

    Ok(data)
}
pub(super) fn execute_set_transition_duration(
    mut data: ProjectData,
    transition: project::TransitionId,
    duration: Tick,
) -> Result<ProjectData> {
    let n = crate::transitions::find(&data, transition)?;
    let mut tr = data
        .scene()
        .timeline
        .transitions
        .get(n)
        .cloned()
        .ok_or(EditError::Invalid("missing transition"))?;
    let bounds = tr.bounds(&data.scene().timeline, &data)?;
    tr.set_duration(duration, bounds)?;
    *data
        .scene_mut()
        .timeline
        .transitions
        .get_mut(n)
        .ok_or(EditError::Invalid("missing transition"))? = tr;

    Ok(data)
}
pub(super) fn execute_remove_transition(
    mut data: ProjectData,
    transition: project::TransitionId,
) -> Result<ProjectData> {
    let n = crate::transitions::find(&data, transition)?;
    data.scene_mut().timeline.transitions.remove(n);

    Ok(data)
}
