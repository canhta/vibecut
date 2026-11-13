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
        params: Default::default(),
    };
    crate::transitions::unlocked(&data.scene().timeline, &tr)?;
    let bounds = tr.bounds(&data.scene().timeline, &data)?;
    tr.set_duration(duration, bounds)?;
    created.push(CreatedId::Transition(tr.id));
    data.scene_mut().timeline.transitions.push(tr);

    Ok(data)
}
/// Copies the source's kind and duration onto every other transition as one candidate.
/// Transitions on locked tracks are left alone and listed in `skipped`; any other
/// refusal (a span with no handles) propagates and the whole edit changes nothing.
/// A target whose kind changes drops its overrides: they are keyed to the old kind.
pub(super) fn execute_apply_to_all(
    mut data: ProjectData,
    transition: project::TransitionId,
    skipped: &mut Vec<project::TransitionId>,
) -> Result<ProjectData> {
    let n = crate::transitions::find(&data, transition)?;
    let source = data
        .scene()
        .timeline
        .transitions
        .get(n)
        .cloned()
        .ok_or(EditError::Invalid("missing transition"))?;
    let targets: Vec<project::TransitionId> = data
        .scene()
        .timeline
        .transitions
        .iter()
        .map(|tr| tr.id)
        .filter(|id| *id != transition)
        .collect();
    for id in targets {
        let n = data
            .scene()
            .timeline
            .transitions
            .iter()
            .position(|tr| tr.id == id)
            .ok_or(EditError::Invalid("missing transition"))?;
        let mut tr = data
            .scene()
            .timeline
            .transitions
            .get(n)
            .cloned()
            .ok_or(EditError::Invalid("missing transition"))?;
        match crate::transitions::unlocked(&data.scene().timeline, &tr) {
            Ok(()) => {}
            Err(EditError::Locked(_)) => {
                skipped.push(id);
                continue;
            }
            Err(error) => return Err(error),
        }
        let bounds = tr.bounds(&data.scene().timeline, &data)?;
        if tr.kind != source.kind {
            tr.kind.clone_from(&source.kind);
            tr.params = Default::default();
        }
        tr.set_duration(source.duration, bounds)?;
        *data
            .scene_mut()
            .timeline
            .transitions
            .get_mut(n)
            .ok_or(EditError::Invalid("missing transition"))? = tr;
    }

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
pub(super) fn execute_set_transition_param(
    mut data: ProjectData,
    transition: project::TransitionId,
    param: &str,
    value: project::TransitionValue,
) -> Result<ProjectData> {
    let n = crate::transitions::find(&data, transition)?;
    data.scene_mut()
        .timeline
        .transitions
        .get_mut(n)
        .ok_or(EditError::Invalid("missing transition"))?
        .set_param(param, value)?;

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
