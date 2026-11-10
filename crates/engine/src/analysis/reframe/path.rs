//! Chooses the followed face of a shot and smooths its crop path.
use super::{
    ReframeReason, ReframeSample, ReframeShot, STEP,
    sampling::{HOLD, Observed, Track},
};
use crate::{analysis::Result, session::wire::ControlError};
use time::{SourceRange, TICKS_PER_SECOND, Tick};

/// A track is stable once it was seen on this many samples (0.5 s), or on half
/// of a shot too short to have them.
const MIN_STABLE: usize = 4;
/// Two stable tracks visible at once on this many samples make a shot ambiguous.
const MIN_OVERLAP: usize = 2;
/// Smoothing (research R1): no movement inside 12 source pixels, otherwise a
/// first-order step with time constant 0.35 s, capped at 768 source px/s.
const DEADBAND: f64 = 12.;
const TAU: f64 = 0.35;
const MAX_SPEED: f64 = 768.;
/// Samples of reflection that extend each end of a shot while filtering.
const PAD: usize = 12;

/// The smoothed crop path of one shot. Position is the crop's top-left corner.
pub(super) fn plan(
    shot: SourceRange,
    observed: &Observed,
    source: [u32; 2],
    size: [f64; 2],
) -> Result<ReframeShot> {
    let n = observed.samples;
    let (w, h) = (f64::from(source[0]), f64::from(source[1]));
    let (cw, ch) = (size[0] * w, size[1] * h);
    let (room_x, room_y) = ((w - cw).max(0.), (h - ch).max(0.));
    let centre = (room_x / 2., room_y / 2.);
    let need = MIN_STABLE.min(n.div_ceil(2)).max(1);
    let stable: Vec<&Track> = observed
        .tracks
        .iter()
        .filter(|t| t.hits.len() >= need)
        .collect();
    let mut reasons = Vec::new();
    let mut overlap = vec![0u8; n];
    for track in &stable {
        for hit in &track.hits {
            let count = overlap.get_mut(hit.sample).ok_or(ControlError::Limit)?;
            *count = count.saturating_add(1);
        }
    }
    if overlap.iter().filter(|c| **c >= 2).count() >= MIN_OVERLAP {
        reasons.push(ReframeReason::Ambiguous);
    }
    let followed = stable
        .iter()
        .copied()
        .fold(None, |best: Option<&Track>, t| match best {
            Some(b) if mean_area(b) >= mean_area(t) => Some(b),
            _ => Some(t),
        });
    let mut targets = vec![centre; n];
    match followed {
        None => reasons.push(ReframeReason::NoSubject),
        Some(track) => {
            let mut seen: Vec<Option<(f64, f64)>> = vec![None; n];
            for hit in &track.hits {
                *seen.get_mut(hit.sample).ok_or(ControlError::Limit)? = Some((
                    (f64::from(hit.cx) - cw / 2.).clamp(0., room_x),
                    (f64::from(hit.cy) - ch / 2.).clamp(0., room_y),
                ));
            }
            let first = seen.iter().flatten().next().copied().unwrap_or(centre);
            let mut last: Option<(usize, (f64, f64))> = None;
            let mut lost = false;
            for (i, target) in targets.iter_mut().enumerate() {
                if let Some(at) = seen.get(i).copied().flatten() {
                    last = Some((i, at));
                }
                *target = match last {
                    // Before the face first appears the crop waits where it will be.
                    None => first,
                    Some((j, at)) if (i - j) as i64 <= HOLD => at,
                    Some(_) => {
                        lost = true;
                        centre
                    }
                };
            }
            if lost {
                reasons.push(ReframeReason::Lost);
            }
        }
    }
    let dt = STEP as f64 / TICKS_PER_SECOND as f64;
    let xs = smooth(&targets.iter().map(|t| t.0).collect::<Vec<_>>(), dt);
    let ys = smooth(&targets.iter().map(|t| t.1).collect::<Vec<_>>(), dt);
    let samples = xs
        .iter()
        .zip(&ys)
        .enumerate()
        .map(|(i, (x, y))| ReframeSample {
            at: Tick(shot.start().0 + i as i64 * STEP),
            x: (x / w).clamp(0., 1. - size[0]),
            y: (y / h).clamp(0., 1. - size[1]),
        })
        .collect();
    Ok(ReframeShot {
        range: shot,
        reasons,
        samples,
    })
}
fn mean_area(track: &Track) -> f64 {
    track.hits.iter().map(|h| f64::from(h.area)).sum::<f64>() / track.hits.len().max(1) as f64
}
fn step(from: f64, to: f64, dt: f64) -> f64 {
    let delta = to - from;
    if delta.abs() <= DEADBAND {
        return from;
    }
    let cap = MAX_SPEED * dt;
    from + ((1. - (-dt / TAU).exp()) * delta).clamp(-cap, cap)
}
/// The filter run forward, then backward over its own output so the path does
/// not lag the subject. Both ends are first extended by an odd reflection of up
/// to `PAD` samples, which lets each pass settle before the shot's own samples
/// instead of ending one full lag away from the subject.
fn smooth(targets: &[f64], dt: f64) -> Vec<f64> {
    let pad = PAD.min(targets.len().saturating_sub(1));
    let at = |i: usize| targets.get(i).copied().unwrap_or_default();
    let last = targets.len().saturating_sub(1);
    let mut signal = Vec::with_capacity(targets.len() + 2 * pad);
    signal.extend((1..=pad).rev().map(|k| 2. * at(0) - at(k)));
    signal.extend_from_slice(targets);
    signal.extend((1..=pad).map(|k| 2. * at(last) - at(last - k)));
    let mut path = Vec::with_capacity(signal.len());
    for (i, target) in signal.iter().enumerate() {
        let value = match path.last() {
            Some(previous) if i > 0 => step(*previous, *target, dt),
            _ => *target,
        };
        path.push(value);
    }
    for i in (0..path.len().saturating_sub(1)).rev() {
        let (Some(next), Some(own)) = (path.get(i + 1).copied(), path.get(i).copied()) else {
            continue;
        };
        if let Some(slot) = path.get_mut(i) {
            *slot = step(next, own, dt);
        }
    }
    path.drain(pad..pad + targets.len()).collect()
}
