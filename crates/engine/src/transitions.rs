use crate::{EditError, Result};
use project::{ProjectData, Timeline, Transition, TransitionId};
use std::collections::BTreeMap;
use time::Tick;

pub(crate) fn find(data: &ProjectData, id: TransitionId) -> Result<usize> {
    let timeline = &data.scene().timeline;
    let n = timeline
        .transitions
        .iter()
        .position(|t| t.id == id)
        .ok_or(EditError::Invalid("missing transition"))?;
    let tr = timeline
        .transitions
        .get(n)
        .ok_or(EditError::Invalid("missing transition"))?;
    unlocked(timeline, tr)?;
    Ok(n)
}
pub(crate) fn unlocked(timeline: &Timeline, tr: &Transition) -> Result<()> {
    let track = timeline
        .iter()
        .find(|t| t.id == tr.track)
        .ok_or(EditError::NoTrack(tr.track))?;
    if track.locked {
        return Err(EditError::Locked(track.id));
    }
    Ok(())
}
/// Output position of each transition's cut, captured before an edit moves clips.
pub(crate) fn cuts(timeline: &Timeline) -> BTreeMap<TransitionId, Tick> {
    timeline
        .transitions
        .iter()
        .filter_map(|tr| Some((tr.id, tr.clips(timeline).ok()?.1.start)))
        .collect()
}
/// Preserve relative placement when the cut moves; remove lost adjacency or
/// exhausted handles and clamp shortened clips before publishing the candidate.
pub(crate) fn revalidate(data: &mut ProjectData, previous: &BTreeMap<TransitionId, Tick>) {
    let mut kept = Vec::new();
    for mut tr in std::mem::take(&mut data.scene_mut().timeline.transitions) {
        let bounds = tr.bounds(&data.scene().timeline, data);
        let Ok(bounds) = bounds else { continue };
        if let Some(old_cut) = previous.get(&tr.id) {
            let shifted = bounds
                .1
                .checked_sub(*old_cut)
                .and_then(|delta| tr.start.checked_add(delta));
            let Some(start) = shifted else { continue };
            tr.start = start;
        }
        if tr.set_duration(tr.duration, bounds).is_ok() {
            kept.push(tr);
        }
    }
    data.scene_mut().timeline.transitions = kept;
}

pub(crate) fn move_span(
    tr: &mut Transition,
    start: Tick,
    bounds: (Tick, Tick, Tick),
) -> Result<()> {
    let (lo, cut, hi) = bounds;
    let min = lo.max(
        cut.checked_sub(tr.duration)
            .ok_or(EditError::Invalid("transition overflow"))?,
    );
    let max = cut.min(
        hi.checked_sub(tr.duration)
            .ok_or(EditError::Invalid("transition overflow"))?,
    );
    if min > max {
        return Err(EditError::Invalid("transition handles"));
    }
    tr.start = start.clamp(min, max);
    tr.alignment = if tr.start == cut {
        project::TransitionAlignment::Start
    } else if tr.end()? == cut {
        project::TransitionAlignment::End
    } else if tr.start.checked_add(Tick(tr.duration.0 / 2)) == Some(cut) {
        project::TransitionAlignment::Center
    } else {
        project::TransitionAlignment::Custom
    };
    Ok(())
}
