//! Expands built-in clip animations into ordinary editable keys on the channels they own.
use super::super::{ItemId, ProjectData, Result, Tick, item};
use crate::session::EditError;
use crate::{AnimationPreset as P, AnimationSlot, ClipAnimations};
use project::{
    Animations, BuiltinPath, Canvas, Channel, Extrapolation::Hold, Item, Keyframe, Param,
    ParamValue::Scalar, TransformField,
};
use std::collections::BTreeSet;
use time::Rounding;

/// Default duration of an In or Out animation: half a second.
pub(crate) const DEFAULT_DURATION: Tick = Tick(127_008_000_000);
/// Slide and drop travel as a fraction of the canvas width or height.
const TRAVEL: f64 = 0.25;

/// Channels the animations write; every other channel is user animation.
const OWNED: [BuiltinPath; 6] = [
    BuiltinPath::Opacity,
    BuiltinPath::TransformPositionX,
    BuiltinPath::TransformPositionY,
    BuiltinPath::TransformScaleX,
    BuiltinPath::TransformScaleY,
    BuiltinPath::TransformRotate,
];
const OPACITY: usize = 0;
const POSITION_X: usize = 1;
const POSITION_Y: usize = 2;
const SCALE_X: usize = 3;
const SCALE_Y: usize = 4;

/// Time-ordered `(item-relative tick, value)` pairs of one channel.
type Keys = Vec<(Tick, f64)>;

/// The keys every owned channel should carry; `None` marks a channel left alone.
#[derive(Debug, Default, PartialEq)]
struct Plan([Option<Keys>; 6]);

/// How far an In look starts, or an Out look ends, from the item's resting state.
#[derive(Clone, Copy)]
struct Look {
    x: f64,
    y: f64,
    scale_x: f64,
    scale_y: f64,
    rotate: f64,
    fade: bool,
}
const fn look(x: f64, y: f64, scale: f64, rotate: f64, fade: bool) -> Look {
    Look {
        x,
        y,
        scale_x: scale,
        scale_y: scale,
        rotate,
        fade,
    }
}
/// A look that squashes or stretches the two axes by different factors.
const fn stretch(scale_x: f64, scale_y: f64, fade: bool) -> Look {
    Look {
        x: 0.0,
        y: 0.0,
        scale_x,
        scale_y,
        rotate: 0.0,
        fade,
    }
}

/// One Combo waypoint: permille of the clip and the offset from the resting state.
#[derive(Clone, Copy)]
struct Waypoint {
    at: i64,
    x: f64,
    y: f64,
    scale: f64,
    rotate: f64,
    opacity: f64,
}
const fn at(at: i64, x: f64, y: f64, scale: f64, rotate: f64, opacity: f64) -> Waypoint {
    Waypoint {
        at,
        x,
        y,
        scale,
        rotate,
        opacity,
    }
}

/// Travel is in the direction the picture moves: a Slide left look enters from the right.
fn edge_look(preset: P) -> Option<Look> {
    Some(match preset {
        P::Fade => look(0.0, 0.0, 0.0, 0.0, true),
        P::SlideLeft => look(-TRAVEL, 0.0, 0.0, 0.0, false),
        P::SlideRight => look(TRAVEL, 0.0, 0.0, 0.0, false),
        P::SlideUp => look(0.0, -TRAVEL, 0.0, 0.0, false),
        P::SlideDown => look(0.0, TRAVEL, 0.0, 0.0, false),
        P::ZoomIn => look(0.0, 0.0, 0.5, 0.0, false),
        P::ZoomOut => look(0.0, 0.0, -0.5, 0.0, false),
        P::Spin => look(0.0, 0.0, 0.0, 180.0, true),
        P::Drop => look(0.0, TRAVEL, 0.0, 0.0, true),
        P::Rise => look(0.0, -TRAVEL, 0.0, 0.0, true),
        P::Pop => look(0.0, 0.0, 0.8, 0.0, true),
        // Narrow and tall as it arrives, settling to the resting proportions.
        P::Stretch => stretch(0.5, -0.5, true),
        P::Swing => look(0.0, 0.0, 0.0, 20.0, true),
        _ => return None,
    })
}

/// Shake in or out: the shake waveform compressed into the edge's duration, on X only.
fn shake_keys(
    index: usize,
    base: f64,
    canvas: &Canvas,
    length: Tick,
    duration: Tick,
    entering: bool,
) -> Result<Option<Keys>> {
    if index != POSITION_X {
        return Ok(None);
    }
    let start = if entering {
        Tick::ZERO
    } else {
        length
            .checked_sub(duration)
            .ok_or(EditError::Invalid("time overflow"))?
    };
    let mut keys = Vec::with_capacity(SHAKE.len());
    for w in SHAKE {
        // `checked_scale` refuses a zero factor, so the first waypoint is the edge start.
        let time = if w.at == 0 {
            start
        } else {
            start + duration.checked_scale(w.at, 1000, Rounding::Nearest)?
        };
        let mut value = base + w.x * f64::from(canvas.width);
        if let Some(field) = transform_field(index) {
            value = field.clamp(value)?;
        }
        keys.push((time, value));
    }
    Ok(Some(keys))
}

/// The keys one In or Out look writes on one channel.
fn edge_parts(
    preset: P,
    index: usize,
    base: f64,
    canvas: &Canvas,
    length: Tick,
    duration: Tick,
    entering: bool,
) -> Result<Option<Keys>> {
    if preset == P::Shake {
        return shake_keys(index, base, canvas, length, duration, entering);
    }
    let look = edge_look(preset).ok_or(EditError::Invalid("not an In or Out look"))?;
    edge_keys(look, index, base, canvas, length, duration, entering)
}

const PULSE: &[Waypoint] = &[
    at(0, 0.0, 0.0, 0.0, 0.0, 1.0),
    at(500, 0.0, 0.0, 0.1, 0.0, 1.0),
    at(1000, 0.0, 0.0, 0.0, 0.0, 1.0),
];
const HEARTBEAT: &[Waypoint] = &[
    at(0, 0.0, 0.0, 0.0, 0.0, 1.0),
    at(150, 0.0, 0.0, 0.12, 0.0, 1.0),
    at(300, 0.0, 0.0, 0.0, 0.0, 1.0),
    at(450, 0.0, 0.0, 0.12, 0.0, 1.0),
    at(600, 0.0, 0.0, 0.0, 0.0, 1.0),
    at(750, 0.0, 0.0, 0.12, 0.0, 1.0),
    at(900, 0.0, 0.0, 0.0, 0.0, 1.0),
    at(1000, 0.0, 0.0, 0.0, 0.0, 1.0),
];
const SWAY: &[Waypoint] = &[
    at(0, 0.0, 0.0, 0.0, 0.0, 1.0),
    at(250, 0.0, 0.0, 0.0, 4.0, 1.0),
    at(750, 0.0, 0.0, 0.0, -4.0, 1.0),
    at(1000, 0.0, 0.0, 0.0, 0.0, 1.0),
];
const FLOAT: &[Waypoint] = &[
    at(0, 0.0, 0.0, 0.0, 0.0, 1.0),
    at(500, 0.0, -0.03, 0.0, 0.0, 1.0),
    at(1000, 0.0, 0.0, 0.0, 0.0, 1.0),
];
const DRIFT_LEFT: &[Waypoint] = &[
    at(0, 0.0, 0.0, 0.0, 0.0, 1.0),
    at(1000, -0.05, 0.0, 0.0, 0.0, 1.0),
];
const DRIFT_RIGHT: &[Waypoint] = &[
    at(0, 0.0, 0.0, 0.0, 0.0, 1.0),
    at(1000, 0.05, 0.0, 0.0, 0.0, 1.0),
];
const PUSH_IN: &[Waypoint] = &[
    at(0, 0.0, 0.0, 0.0, 0.0, 1.0),
    at(1000, 0.0, 0.0, 0.15, 0.0, 1.0),
];
const PULL_OUT: &[Waypoint] = &[
    at(0, 0.0, 0.0, 0.15, 0.0, 1.0),
    at(1000, 0.0, 0.0, 0.0, 0.0, 1.0),
];
const SHAKE: &[Waypoint] = &[
    at(0, 0.0, 0.0, 0.0, 0.0, 1.0),
    at(125, 0.012, 0.0, 0.0, 0.0, 1.0),
    at(250, -0.012, 0.0, 0.0, 0.0, 1.0),
    at(375, 0.012, 0.0, 0.0, 0.0, 1.0),
    at(500, -0.012, 0.0, 0.0, 0.0, 1.0),
    at(625, 0.012, 0.0, 0.0, 0.0, 1.0),
    at(750, -0.012, 0.0, 0.0, 0.0, 1.0),
    at(875, 0.012, 0.0, 0.0, 0.0, 1.0),
    at(1000, 0.0, 0.0, 0.0, 0.0, 1.0),
];
const FLICKER: &[Waypoint] = &[
    at(0, 0.0, 0.0, 0.0, 0.0, 1.0),
    at(150, 0.0, 0.0, 0.0, 0.0, 0.4),
    at(300, 0.0, 0.0, 0.0, 0.0, 1.0),
    at(500, 0.0, 0.0, 0.0, 0.0, 0.5),
    at(700, 0.0, 0.0, 0.0, 0.0, 1.0),
    at(850, 0.0, 0.0, 0.0, 0.0, 0.6),
    at(1000, 0.0, 0.0, 0.0, 0.0, 1.0),
];
const TURN: &[Waypoint] = &[
    at(0, 0.0, 0.0, 0.0, 0.0, 1.0),
    at(1000, 0.0, 0.0, 0.0, 360.0, 1.0),
];

fn combo_table(preset: P) -> Option<&'static [Waypoint]> {
    Some(match preset {
        P::Pulse => PULSE,
        P::Heartbeat => HEARTBEAT,
        P::Sway => SWAY,
        P::Float => FLOAT,
        P::DriftLeft => DRIFT_LEFT,
        P::DriftRight => DRIFT_RIGHT,
        P::PushIn => PUSH_IN,
        P::PullOut => PULL_OUT,
        P::Flicker => FLICKER,
        P::Turn => TURN,
        _ => return None,
    })
}

fn transform_field(index: usize) -> Option<TransformField> {
    match index {
        POSITION_X => Some(TransformField::PositionX),
        POSITION_Y => Some(TransformField::PositionY),
        SCALE_X => Some(TransformField::ScaleX),
        SCALE_Y => Some(TransformField::ScaleY),
        OPACITY => None,
        _ => Some(TransformField::Rotate),
    }
}

fn default_of(index: usize) -> f64 {
    if index == OPACITY {
        1.0
    } else {
        transform_field(index).map_or(0.0, TransformField::default_value)
    }
}

/// Current scalar base of `path`, or the channel's documented default.
fn scalar_base(target: &Item, path: BuiltinPath, default: f64) -> Result<f64> {
    match target.animations.get(&path.into()) {
        None => Ok(default),
        Some(Param::Value(channel)) => match channel.base() {
            Scalar(base) if base.is_finite() => Ok(*base),
            _ => Err(EditError::Invalid("clip animation base")),
        },
        _ => Err(EditError::Invalid("clip animation base")),
    }
}

fn resting(target: &Item) -> Result<[f64; 6]> {
    let mut out = [0.0; 6];
    for ((index, path), slot) in OWNED.into_iter().enumerate().zip(out.iter_mut()) {
        *slot = scalar_base(target, path, default_of(index))?;
    }
    if !(0.0..=1.0).contains(&out[OPACITY]) {
        return Err(EditError::Invalid("clip opacity base"));
    }
    Ok(out)
}

/// The scalar keys each owned channel carries now.
fn keys_of(target: &Item) -> [Keys; 6] {
    OWNED.map(|path| match target.animations.get(&path.into()) {
        Some(Param::Value(channel)) => channel
            .keys()
            .iter()
            .filter_map(|key| match &key.value {
                Scalar(value) => Some((key.time, *value)),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    })
}

fn edge_keys(
    look: Look,
    index: usize,
    base: f64,
    canvas: &Canvas,
    length: Tick,
    duration: Tick,
    entering: bool,
) -> Result<Option<Keys>> {
    let sign = if entering { -1.0 } else { 1.0 };
    let outer = match index {
        OPACITY => look.fade.then_some(0.0),
        POSITION_X => (look.x != 0.0).then(|| base + sign * look.x * f64::from(canvas.width)),
        POSITION_Y => (look.y != 0.0).then(|| base + sign * look.y * f64::from(canvas.height)),
        SCALE_X => (look.scale_x != 0.0).then_some(base * (1.0 + sign * look.scale_x)),
        SCALE_Y => (look.scale_y != 0.0).then_some(base * (1.0 + sign * look.scale_y)),
        _ => (look.rotate != 0.0).then_some(base + sign * look.rotate),
    };
    let Some(mut outer) = outer else {
        return Ok(None);
    };
    if let Some(field) = transform_field(index) {
        outer = field.clamp(outer)?;
    }
    Ok(Some(if entering {
        vec![(Tick::ZERO, outer), (duration, base)]
    } else {
        let start = length
            .checked_sub(duration)
            .ok_or(EditError::Invalid("time overflow"))?;
        vec![(start, base), (length, outer)]
    }))
}

fn combo_keys(
    table: &[Waypoint],
    index: usize,
    base: f64,
    canvas: &Canvas,
    length: Tick,
) -> Result<Option<Keys>> {
    let touched = table.iter().any(|w| match index {
        OPACITY => w.opacity != 1.0,
        POSITION_X => w.x != 0.0,
        POSITION_Y => w.y != 0.0,
        SCALE_X | SCALE_Y => w.scale != 0.0,
        _ => w.rotate != 0.0,
    });
    if !touched {
        return Ok(None);
    }
    let mut keys = Vec::with_capacity(table.len());
    for w in table {
        let time = if w.at == 0 {
            Tick::ZERO
        } else {
            length.checked_scale(w.at, 1000, Rounding::Nearest)?
        };
        let mut value = match index {
            OPACITY => base * w.opacity,
            POSITION_X => base + w.x * f64::from(canvas.width),
            POSITION_Y => base + w.y * f64::from(canvas.height),
            SCALE_X | SCALE_Y => base * (1.0 + w.scale),
            _ => base + w.rotate,
        };
        if let Some(field) = transform_field(index) {
            value = field.clamp(value)?;
        }
        keys.push((time, value));
    }
    Ok(Some(keys))
}

/// Join the In and Out keys of one channel; they meet only at one shared resting key.
fn merge(parts: Vec<Keys>) -> Result<Keys> {
    let mut out: Keys = Vec::new();
    for (time, value) in parts.into_iter().flatten() {
        match out.last() {
            Some(&(last, held)) if last == time && held == value => {}
            Some(&(last, _)) if last >= time => {
                return Err(EditError::Invalid("animations overlap"));
            }
            _ => out.push((time, value)),
        }
    }
    Ok(out)
}

fn plan(anim: &ClipAnimations, bases: &[f64; 6], length: Tick, canvas: &Canvas) -> Result<Plan> {
    let mut out = Plan::default();
    for ((index, base), slot) in bases.iter().copied().enumerate().zip(out.0.iter_mut()) {
        let mut parts: Vec<Keys> = Vec::new();
        if let Some((preset, duration)) = anim.enter {
            parts.extend(edge_parts(
                preset, index, base, canvas, length, duration, true,
            )?);
        }
        if let Some((preset, duration)) = anim.exit {
            parts.extend(edge_parts(
                preset, index, base, canvas, length, duration, false,
            )?);
        }
        if let Some(preset) = anim.combo {
            let table = combo_table(preset).ok_or(EditError::Invalid("not a Combo look"))?;
            parts.extend(combo_keys(table, index, base, canvas, length)?);
        }
        if !parts.is_empty() {
            *slot = Some(merge(parts)?);
        }
    }
    Ok(out)
}

/// Clamp the durations to the clip: each lasts at least one frame and at most the clip, and
/// when both edges are set the later one (Out) shrinks so the two never overlap. The clip
/// must last two frames or more.
fn fit(anim: &mut ClipAnimations, length: Tick, frame: Tick) {
    let exit_set = anim.exit.is_some();
    if let Some((_, duration)) = &mut anim.enter {
        let top = Tick(length.0 - if exit_set { frame.0 } else { 0 });
        *duration = (*duration).max(frame).min(top);
    }
    let taken = anim.enter.map_or(0, |(_, duration)| duration.0);
    if let Some((_, duration)) = &mut anim.exit {
        *duration = (*duration).max(frame).min(Tick(length.0 - taken));
    }
}

fn param(base: f64, keys: &[(Tick, f64)]) -> Result<Param> {
    Ok(Param::Value(Channel::new(
        Scalar(base),
        keys.iter()
            .map(|(time, value)| Keyframe::new(*time, Scalar(*value)))
            .collect(),
        Hold,
        Hold,
    )?))
}

/// Write the new keys; a channel only the old animations owned returns to its bare base.
fn write(target: &mut Item, old: &Plan, new: &Plan, bases: &[f64; 6]) -> Result<()> {
    let rows = OWNED
        .into_iter()
        .enumerate()
        .zip(new.0.iter().zip(&old.0))
        .zip(bases.iter().copied());
    for (((index, path), (new, old)), base) in rows {
        match (new, old) {
            (Some(keys), _) => target.animations.set(path.into(), param(base, keys)?)?,
            (None, Some(_)) => {
                if base == default_of(index) {
                    target.animations.remove(&path.into());
                } else {
                    target.animations.set(path.into(), param(base, &[])?)?;
                }
            }
            (None, None) => {}
        }
    }
    Ok(())
}

/// The keys one look writes onto a bare sample item that rests at every default and lasts
/// `length` on `canvas`, as an ordinary key set a preview evaluates. Same expansion, same
/// clamps as `clip.set_animation`; nothing is stored.
pub(crate) fn sample_animations(
    slot: AnimationSlot,
    preset: P,
    duration: Tick,
    length: Tick,
    canvas: &Canvas,
) -> Result<Animations> {
    let anim = match slot {
        AnimationSlot::In => ClipAnimations {
            enter: Some((preset, duration)),
            ..ClipAnimations::default()
        },
        AnimationSlot::Out => ClipAnimations {
            exit: Some((preset, duration)),
            ..ClipAnimations::default()
        },
        AnimationSlot::Combo => ClipAnimations {
            combo: Some(preset),
            ..ClipAnimations::default()
        },
    };
    if anim
        .enter
        .is_some_and(|(_, held)| held <= Tick::ZERO || held > length)
        || anim
            .exit
            .is_some_and(|(_, held)| held <= Tick::ZERO || held > length)
    {
        return Err(EditError::Invalid("clip animation duration"));
    }
    let bases: [f64; 6] = std::array::from_fn(default_of);
    let planned = plan(&anim, &bases, length, canvas)?;
    let mut out = Animations::default();
    for ((path, keys), base) in OWNED.into_iter().zip(&planned.0).zip(bases) {
        if let Some(keys) = keys {
            out.set(path.into(), param(base, keys)?)?;
        }
    }
    Ok(out)
}

impl ClipAnimations {
    /// The animations the item's keys spell out. A channel counts only when its keys equal
    /// what a look would write, so hand-edited keys read as no animation, and keys on other
    /// channels are ignored. Of several readings the one explaining the most channels wins.
    pub fn of(target: &Item, canvas: &Canvas) -> Self {
        Self::detect(target, canvas).unwrap_or_default()
    }

    fn detect(target: &Item, canvas: &Canvas) -> Result<Self> {
        let length = target.duration()?;
        let bases = resting(target)?;
        let current = keys_of(target);
        let times: BTreeSet<Tick> = current.iter().flatten().map(|(time, _)| *time).collect();
        if times.is_empty() {
            return Ok(Self::default());
        }
        let agrees = |anim: &Self, whole: bool| {
            plan(anim, &bases, length, canvas).is_ok_and(|plan| {
                plan.0.iter().zip(&current).all(|(want, have)| {
                    want.as_ref().is_none_or(|keys| {
                        if whole {
                            keys == have
                        } else {
                            keys.iter().all(|key| have.contains(key))
                        }
                    })
                })
            })
        };
        let mut enters = vec![None];
        let mut exits = vec![None];
        for preset in P::EDGE {
            for time in &times {
                if *time > Tick::ZERO && *time <= length {
                    let enter = Self {
                        enter: Some((preset, *time)),
                        ..Self::default()
                    };
                    if agrees(&enter, false) {
                        enters.push(enter.enter);
                    }
                }
                if let Some(duration) = length.checked_sub(*time)
                    && duration > Tick::ZERO
                {
                    let exit = Self {
                        exit: Some((preset, duration)),
                        ..Self::default()
                    };
                    if agrees(&exit, false) {
                        exits.push(exit.exit);
                    }
                }
            }
        }
        let mut best = (0, 0, Self::default());
        let mut consider = |anim: Self| {
            if !agrees(&anim, true) {
                return;
            }
            let slots = usize::from(anim.enter.is_some())
                + usize::from(anim.exit.is_some())
                + usize::from(anim.combo.is_some());
            let channels = plan(&anim, &bases, length, canvas)
                .map_or(0, |plan| plan.0.iter().flatten().count());
            if (slots, channels) > (best.0, best.1) {
                best = (slots, channels, anim);
            }
        };
        for enter in &enters {
            for exit in &exits {
                consider(Self {
                    enter: *enter,
                    exit: *exit,
                    combo: None,
                });
            }
        }
        for preset in P::COMBO {
            consider(Self {
                combo: Some(preset),
                ..Self::default()
            });
        }
        Ok(best.2)
    }
}

pub(in super::super) fn execute_set_clip_animation(
    mut data: ProjectData,
    id: ItemId,
    slot: AnimationSlot,
    preset: Option<P>,
    duration: Option<Tick>,
) -> Result<ProjectData> {
    let canvas = data.canvas.clone();
    let target = item(&mut data, id)?; // Locks also refuse exact repeats.
    if !target.content.supports_animation() {
        return Err(EditError::Invalid("clip animation requires a visual item"));
    }
    let length = target.duration()?;
    let frame = canvas.fps.tick_of(1)?;
    if length.0 < frame.0.saturating_mul(2) {
        return Err(EditError::Invalid("clip animation requires two frames"));
    }
    let current = ClipAnimations::of(target, &canvas);
    let mut wanted = current;
    match (slot, preset) {
        (AnimationSlot::Combo, Some(preset)) => {
            if duration.is_some() || combo_table(preset).is_none() {
                return Err(EditError::Invalid(
                    "combo takes a combo look and no duration",
                ));
            }
            wanted = ClipAnimations {
                combo: Some(preset),
                ..ClipAnimations::default()
            };
        }
        (AnimationSlot::Combo, None) => {
            if duration.is_some() {
                return Err(EditError::Invalid("combo takes no duration"));
            }
            wanted.combo = None;
        }
        (edge, Some(preset)) => {
            if !P::EDGE.contains(&preset) {
                return Err(EditError::Invalid("In and Out take an edge look"));
            }
            let held = if edge == AnimationSlot::In {
                current.enter
            } else {
                current.exit
            };
            let length_of = duration
                .or(held.map(|(_, held)| held))
                .unwrap_or(DEFAULT_DURATION);
            wanted.combo = None;
            if edge == AnimationSlot::In {
                wanted.enter = Some((preset, length_of));
            } else {
                wanted.exit = Some((preset, length_of));
            }
        }
        (edge, None) => {
            if duration.is_some() {
                return Err(EditError::Invalid("a duration needs a look"));
            }
            if edge == AnimationSlot::In {
                wanted.enter = None;
            } else {
                wanted.exit = None;
            }
        }
    }
    fit(&mut wanted, length, frame);
    if wanted == current {
        return Ok(data);
    }
    let bases = resting(target)?;
    let old = plan(&current, &bases, length, &canvas)?;
    let new = plan(&wanted, &bases, length, &canvas)?;
    let have = keys_of(target);
    for ((before, after), keys) in old.0.iter().zip(&new.0).zip(&have) {
        let touched = before.is_some() || after.is_some();
        if touched && !keys.is_empty() && Some(keys) != before.as_ref() {
            return Err(EditError::Invalid(
                "remove animation keys before changing animation",
            ));
        }
    }
    write(target, &old, &new, &bases)?;
    Ok(data)
}

/// Re-fit the item's animations to its new length after a trim has shifted its keys:
/// durations clamp to the clip, Out shrinks before In, and a clip under two frames loses
/// them. `before` and `before_length` are read from the item ahead of the trim.
pub(in super::super) fn refit(
    target: &mut Item,
    before: ClipAnimations,
    before_length: Tick,
    canvas: &Canvas,
) -> Result<()> {
    if before == ClipAnimations::default() {
        return Ok(());
    }
    let length = target.duration()?;
    let frame = canvas.fps.tick_of(1)?;
    let bases = resting(target)?;
    let old = plan(&before, &bases, before_length, canvas)?;
    let mut wanted = before;
    if length.0 < frame.0.saturating_mul(2) {
        wanted = ClipAnimations::default();
    } else {
        fit(&mut wanted, length, frame);
    }
    let new = plan(&wanted, &bases, length, canvas)?;
    write(target, &old, &new, &bases)
}
