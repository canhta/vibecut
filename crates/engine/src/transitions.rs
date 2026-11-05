use project::{Sequence, Track, Transition, TransitionAlign, TransitionId};
use time::Tick;

use crate::{Edge, EditError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    Center,
    Start,
    End,
    Custom,
}

impl Alignment {
    pub const MENU: [Alignment; 3] = [Alignment::Center, Alignment::Start, Alignment::End];

    pub fn id(self) -> &'static str {
        match self {
            Alignment::Center => "center",
            Alignment::Start => "start",
            Alignment::End => "end",
            Alignment::Custom => "custom",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Alignment::Center => "Center at Cut",
            Alignment::Start => "Start at Cut",
            Alignment::End => "End at Cut",
            Alignment::Custom => "Custom Start",
        }
    }

    pub fn parse(s: &str) -> Option<Alignment> {
        Alignment::MENU.into_iter().find(|a| a.id().eq_ignore_ascii_case(s) || a.label().eq_ignore_ascii_case(s))
    }
}

pub fn bounds(track: &Track, tr: &Transition) -> Option<(Tick, Tick, Tick)> {
    let from = tr.from.and_then(|c| track.item(c));
    let to = tr.to.and_then(|c| track.item(c));
    match (from, to) {
        (Some(f), Some(t)) => Some((f.start, t.start, t.end())),
        (None, Some(t)) => Some((t.start, t.start, t.end())),
        (Some(f), None) => Some((f.start, f.end(), f.end())),
        (None, None) => None,
    }
}

pub fn alignment(tr: &Transition, cut: Tick, frame: Tick) -> Alignment {
    match (tr.from, tr.to) {
        (None, Some(_)) => return Alignment::Start,
        (Some(_), None) => return Alignment::End,
        _ => {}
    }
    if tr.start == cut {
        Alignment::Start
    } else if tr.end() == cut {
        Alignment::End
    } else if (tr.start.0.saturating_mul(2).saturating_add(tr.duration.0) - cut.0.saturating_mul(2)).abs() <= frame.0.max(0) {
        Alignment::Center
    } else {
        Alignment::Custom
    }
}

fn find(seq: &Sequence, id: TransitionId) -> Option<(&Track, &Transition)> {
    seq.all_tracks().find_map(|t| t.transitions.iter().find(|x| x.id == id).map(|x| (t, x)))
}

pub fn start_for(seq: &Sequence, id: TransitionId, duration: Tick, align: Alignment, frame: Tick, snap: impl Fn(Tick) -> Tick) -> Option<Tick> {
    let (track, tr) = find(seq, id)?;
    let (_, cut, _) = bounds(track, tr)?;
    let one_sided = alignment(tr, cut, frame);
    let align = match one_sided {
        Alignment::Start | Alignment::End if tr.from.is_none() || tr.to.is_none() => one_sided,
        _ => align,
    };
    Some(match align {
        Alignment::Start => cut,
        Alignment::End => cut - duration,
        Alignment::Center => snap(cut - duration.mul_ratio(1, 2)),
        Alignment::Custom => snap(tr.start + tr.duration.mul_ratio(1, 2) - duration.mul_ratio(1, 2)),
    })
}

pub fn set_span(seq: &mut Sequence, id: TransitionId, start: Tick, duration: Tick, min: Tick, frame: Tick) -> Result<()> {
    let (track, tr) = find(seq, id).ok_or_else(|| EditError::Other(format!("no transition {}", id.0)))?;
    if track.locked {
        return Err(EditError::Locked);
    }
    let (lo, cut, hi) = bounds(track, tr).ok_or_else(|| EditError::Other("the transition's clips are gone".into()))?;
    if duration < min.max(Tick(1)) {
        return Err(EditError::Other("a transition lasts at least one frame".into()));
    }
    let end = start.0.checked_add(duration.0).map(Tick).ok_or_else(|| EditError::Other("transition out of range".into()))?;
    if start < lo || end > hi {
        return Err(EditError::Other("a transition can't extend past the clips it joins".into()));
    }
    let on_cut = match (tr.from, tr.to) {
        (None, Some(_)) => start == cut,
        (Some(_), None) => end == cut,
        _ => start <= cut && cut <= end,
    };
    if !on_cut {
        return Err(EditError::Other(if tr.from.is_some() && tr.to.is_some() {
            "a transition between two clips must stay over their cut".into()
        } else {
            "a one-sided transition stays on its clip's edge".into()
        }));
    }
    let tid = track.id;
    let Some(x) = seq.track_mut(tid).and_then(|t| t.transitions.iter_mut().find(|x| x.id == id)) else {
        return Err(EditError::Other(format!("no transition {}", id.0)));
    };
    x.start = start;
    x.duration = duration;
    x.align = match alignment(x, cut, frame) {
        Alignment::Start => TransitionAlign::StartAtCut,
        Alignment::End => TransitionAlign::EndAtCut,
        Alignment::Center | Alignment::Custom => TransitionAlign::CenterAtCut,
    };
    Ok(())
}

pub fn drag_span(track: &Track, tr: &Transition, edge: Option<Edge>, delta: Tick, frame: Tick) -> Option<(Tick, Tick)> {
    let (lo, cut, hi) = bounds(track, tr)?;
    let frame = frame.max(Tick(1));
    let (s0, e0, d0) = (tr.start, tr.end(), tr.duration);
    let one_sided = tr.from.is_none() || tr.to.is_none();
    let unchanged = Some((s0, d0));
    match edge {
        None if one_sided => unchanged,
        None => {
            let (min_s, max_s) = (lo.max(cut - d0), (hi - d0).min(cut));
            if min_s > max_s { unchanged } else { Some(((s0 + delta).clamp(min_s, max_s), d0)) }
        }
        Some(Edge::In) if tr.from.is_none() => unchanged,
        Some(Edge::In) => {
            let max_s = cut.min(e0 - frame).max(lo);
            let st = (s0 + delta).clamp(lo, max_s);
            Some((st, e0 - st))
        }
        Some(Edge::Out) if tr.to.is_none() => unchanged,
        Some(Edge::Out) => {
            let min_e = cut.max(s0 + frame).min(hi);
            let en = (e0 + delta).clamp(min_e, hi);
            Some((s0, en - s0))
        }
    }
}

pub fn remove(seq: &mut Sequence, ids: &[TransitionId]) -> Result<usize> {
    if seq.all_tracks().any(|t| t.locked && t.transitions.iter().any(|x| ids.contains(&x.id))) {
        return Err(EditError::Locked);
    }
    let mut n = 0;
    for t in seq.all_tracks_mut() {
        let before = t.transitions.len();
        t.transitions.retain(|x| !ids.contains(&x.id));
        n += before - t.transitions.len();
    }
    if n == 0 { Err(EditError::Nothing) } else { Ok(n) }
}

