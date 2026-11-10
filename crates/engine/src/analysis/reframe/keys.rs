//! Turns a cached source-tick path into the crop keys of one timeline item.
use super::{ReframeAnalysis, ReframeReason, ReframeSample, ReframeShot};
use crate::{
    EditError,
    analysis::{Error as AnalysisError, Result},
    session::wire::ControlError,
};
use project::{
    Channel, Crop, Extrapolation, Item, ItemParams, Keyframe, MAX_CHANNEL_KEYS, ParamValue, Segment,
};
use time::Tick;

/// Keys for `crop.x`, `crop.y` and `crop.auto` of one item, in item-relative
/// output ticks, plus the static crop that carries the constant window size.
/// Nothing here touches a project.
#[derive(Clone, Debug, PartialEq)]
pub struct ReframeKeys {
    /// Constant size from the target aspect; its position is the first key.
    pub crop: Crop,
    pub x: Channel,
    pub y: Channel,
    /// True at the item start and at every cut inside the item, held (`Step`).
    pub auto: Channel,
    pub shots: Vec<ShotKeys>,
}
/// What the item's shot got: the tick it starts at and its position key counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShotKeys {
    pub start: Tick,
    pub x_keys: usize,
    pub y_keys: usize,
    pub reasons: Vec<ReframeReason>,
}
/// Converts the shots under `item` to its key set. Output ticks come from
/// `Item::output_time_of` (ceiling) and the start value from `source_time_at`
/// (floor), so two items over one source span each get their own keys from the
/// same analysis. The last key before a cut steps, so nothing interpolates
/// across it. Keys whose removal moves the interpolated position by less than
/// half a source pixel are dropped in one forward pass. A channel above the
/// per-channel key limit is `Limit`, never a truncation.
pub fn emit(analysis: &ReframeAnalysis, item: &Item) -> Result<ReframeKeys> {
    if !matches!(item.content, ItemParams::Video(_)) {
        return Err(AnalysisError::from(EditError::Invalid(
            "reframe needs a video item",
        )));
    }
    let source = item.source_range;
    let [cw, ch] = analysis.size;
    if source.start() < analysis.range.start()
        || source.end() > analysis.range.end()
        || !(cw > 0. && cw <= 1. && ch > 0. && ch <= 1.)
    {
        return Err(AnalysisError::from(EditError::Invalid(
            "item outside the reframe analysis",
        )));
    }
    let duration = item.duration()?;
    let local = |at: Tick| -> Result<Tick> {
        item.output_time_of(at)?
            .checked_sub(item.start)
            .ok_or(AnalysisError::from(ControlError::Limit))
    };
    let mut shots: Vec<(Tick, &ReframeShot)> = Vec::new();
    for shot in &analysis.shots {
        if shot.range.end() <= source.start() || shot.range.start() >= source.end() {
            continue;
        }
        let at = if shot.range.start() <= source.start() {
            Tick::ZERO
        } else {
            local(shot.range.start())?
        };
        if at >= duration {
            continue;
        }
        // A shot shorter than one output tick has no keys; the next one wins.
        if shots.last().is_some_and(|(previous, _)| *previous >= at) {
            shots.pop();
        }
        shots.push((at, shot));
    }
    if shots.is_empty() {
        return Err(AnalysisError::from(EditError::Invalid(
            "no reframe shot under the item",
        )));
    }
    let origin = item.source_time_at(item.start)?;
    let (max_x, max_y) = (1. - cw, 1. - ch);
    let eps = (
        0.5 / f64::from(analysis.source[0].max(1)),
        0.5 / f64::from(analysis.source[1].max(1)),
    );
    let (mut xs, mut ys, mut auto, mut meta) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (index, (start, shot)) in shots.iter().enumerate() {
        let end = shots.get(index + 1).map_or(duration, |(next, _)| *next);
        let anchor = if index == 0 {
            origin
        } else {
            shot.range.start()
        };
        let (x0, y0) = value_at(&shot.samples, anchor)?;
        let mut points = vec![(*start, x0, y0)];
        for sample in shot.samples.iter().filter(|s| s.at > anchor) {
            let at = local(sample.at)?;
            if at >= end {
                break;
            }
            if points.last().is_some_and(|(last, ..)| *last >= at) {
                continue;
            }
            points.push((at, sample.x, sample.y));
        }
        let clamp = |v: f64, max: f64| {
            if v.is_finite() {
                Ok(v.clamp(0., max))
            } else {
                Err(AnalysisError::from(EditError::Invalid(
                    "reframe path value",
                )))
            }
        };
        let times: Vec<Tick> = points.iter().map(|p| p.0).collect();
        let column = |pick: fn(&(Tick, f64, f64)) -> f64, max: f64| {
            points
                .iter()
                .map(|p| clamp(pick(p), max))
                .collect::<Result<Vec<f64>>>()
        };
        let last_shot = index + 1 == shots.len();
        let mut counts = [0; 2];
        for (channel, (values, eps)) in [
            (column(|p| p.1, max_x)?, eps.0),
            (column(|p| p.2, max_y)?, eps.1),
        ]
        .into_iter()
        .enumerate()
        {
            let kept = simplify(&times, &values, eps);
            counts[channel] = kept.len();
            let out = if channel == 0 { &mut xs } else { &mut ys };
            for (n, i) in kept.iter().enumerate() {
                let (Some(time), Some(value)) = (times.get(*i), values.get(*i)) else {
                    return Err(ControlError::Limit.into());
                };
                let mut key = Keyframe::new(*time, ParamValue::Scalar(*value));
                if n + 1 == kept.len() && !last_shot {
                    key.segment = Segment::Step;
                }
                out.push(key);
            }
        }
        let mut flag = Keyframe::new(*start, ParamValue::Bool(true));
        flag.segment = Segment::Step;
        auto.push(flag);
        meta.push(ShotKeys {
            start: *start,
            x_keys: counts[0],
            y_keys: counts[1],
            reasons: shot.reasons.clone(),
        });
    }
    if xs.len() > MAX_CHANNEL_KEYS || ys.len() > MAX_CHANNEL_KEYS {
        return Err(ControlError::Limit.into());
    }
    let first = |keys: &[Keyframe]| match keys.first().map(|k| &k.value) {
        Some(ParamValue::Scalar(v)) => Ok(*v),
        _ => Err(AnalysisError::from(EditError::Invalid(
            "reframe path is empty",
        ))),
    };
    let mut crop = Crop {
        x: first(&xs)?,
        y: first(&ys)?,
        width: cw,
        height: ch,
    };
    // `x + width` can round one ulp past the frame at the far edge.
    while !crop.valid() && crop.x > 0. {
        crop.x = crop.x.next_down();
    }
    while !crop.valid() && crop.y > 0. {
        crop.y = crop.y.next_down();
    }
    let channel = |base: ParamValue, keys: Vec<Keyframe>| {
        Channel::new(base, keys, Extrapolation::Hold, Extrapolation::Hold)
    };
    Ok(ReframeKeys {
        crop,
        x: channel(ParamValue::Scalar(crop.x), xs)?,
        y: channel(ParamValue::Scalar(crop.y), ys)?,
        auto: channel(ParamValue::Bool(true), auto)?,
        shots: meta,
    })
}
/// The path at a source tick: linear between samples, held outside them.
fn value_at(samples: &[ReframeSample], at: Tick) -> Result<(f64, f64)> {
    let i = samples.partition_point(|s| s.at <= at);
    let (Some(first), Some(last)) = (samples.first(), samples.last()) else {
        return Err(AnalysisError::from(EditError::Invalid(
            "reframe shot has no samples",
        )));
    };
    if i == 0 {
        return Ok((first.x, first.y));
    }
    let (Some(a), Some(b)) = (samples.get(i - 1), samples.get(i)) else {
        return Ok((last.x, last.y));
    };
    let f = (at.0 - a.at.0) as f64 / (b.at.0 - a.at.0) as f64;
    Ok((a.x + (b.x - a.x) * f, a.y + (b.y - a.y) * f))
}
/// Indexes to keep: both ends, and any key whose removal (with every key
/// already skipped since the last kept one) moves the line by `eps` or more.
fn simplify(times: &[Tick], values: &[f64], eps: f64) -> Vec<usize> {
    let n = values.len().min(times.len());
    if n <= 2 {
        return (0..n).collect();
    }
    let at = |i: usize| times.get(i).map_or(0., |t| t.0 as f64);
    let value = |i: usize| values.get(i).copied().unwrap_or(0.);
    let mut kept = vec![0];
    let mut anchor = 0;
    for k in 1..n - 1 {
        let (t0, v0, t1, v1) = (at(anchor), value(anchor), at(k + 1), value(k + 1));
        let line = |j: usize| v0 + (v1 - v0) * (at(j) - t0) / (t1 - t0);
        if (anchor + 1..=k).all(|j| (line(j) - value(j)).abs() < eps) {
            continue;
        }
        kept.push(k);
        anchor = k;
    }
    kept.push(n - 1);
    kept
}
