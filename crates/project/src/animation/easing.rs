//! Easing presets and their expansion into explicit segment handles.
use super::keyframes::{Channel, CurveHandle, Keyframe, Segment, TangentMode};
use super::params::ParamValue;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use time::Tick;

/// A normalized cubic Bezier `[x1, y1, x2, y2]` over one keyframe segment: x is
/// the fraction of the segment's time, y the fraction of its value change.
/// `x1` and `x2` lie in `0..=1`; y may overshoot.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct EasingCurve(pub [f64; 4]);

/// Tolerance at which a curve counts as the straight line.
const LINEAR_EPSILON: f64 = 1e-6;
/// Vertical extent used when both keys of a segment hold the same value.
const FALLBACK_SPAN: f64 = 1.0;

impl EasingCurve {
    pub const LINEAR: Self = Self([0.0, 0.0, 1.0, 1.0]);

    pub fn validate(self) -> Result<()> {
        let [x1, y1, x2, y2] = self.0;
        if ![x1, y1, x2, y2].iter().all(|v| v.is_finite())
            || !(0.0..=1.0).contains(&x1)
            || !(0.0..=1.0).contains(&x2)
        {
            return Err(Error::Invalid("easing curve"));
        }
        Ok(())
    }
    pub fn is_linear(self) -> bool {
        let [x1, y1, x2, y2] = self.0;
        x1.abs() <= LINEAR_EPSILON
            && y1.abs() <= LINEAR_EPSILON
            && (x2 - 1.0).abs() <= LINEAR_EPSILON
            && (y2 - 1.0).abs() <= LINEAR_EPSILON
    }
}

/// The named easing choices. Control points are the permitted implementation
/// reference's built-in table (`easing-presets.ts` lines 13-18), stored as data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EasingPreset {
    Smooth,
    EaseOut,
    EaseIn,
    InOut,
    Pop,
    Linear,
}
impl EasingPreset {
    /// Display order of the popover's preset row.
    pub const ALL: [Self; 6] = [
        Self::Smooth,
        Self::EaseOut,
        Self::EaseIn,
        Self::InOut,
        Self::Pop,
        Self::Linear,
    ];
    pub const fn id(self) -> &'static str {
        match self {
            Self::Smooth => "smooth",
            Self::EaseOut => "ease_out",
            Self::EaseIn => "ease_in",
            Self::InOut => "in_out",
            Self::Pop => "pop",
            Self::Linear => "linear",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| preset.id() == id)
    }
    pub const fn curve(self) -> EasingCurve {
        EasingCurve(match self {
            Self::Smooth => [0.25, 0.1, 0.25, 1.0],
            Self::EaseOut => [0.0, 0.0, 0.2, 1.0],
            Self::EaseIn => [0.8, 0.0, 1.0, 1.0],
            Self::InOut => [0.4, 0.0, 0.2, 1.0],
            Self::Pop => [0.175, 0.885, 0.32, 1.275],
            Self::Linear => [0.0, 0.0, 1.0, 1.0],
        })
    }
}

fn scalar(key: &Keyframe) -> Option<f64> {
    match key.value {
        ParamValue::Scalar(value) => Some(value),
        _ => None,
    }
}
fn ticks(a: Tick, b: Tick) -> f64 {
    (i128::from(a.0) - i128::from(b.0)) as f64
}
fn relabel(key: &mut Keyframe) {
    key.tangent = if key.incoming.is_some() || key.outgoing.is_some() {
        TangentMode::Broken
    } else {
        TangentMode::Flat
    };
}

impl Channel {
    /// Vertical scale of the segment leaving `index`: its own value change, or
    /// when flat the nearest non-flat segment's span, else 1.
    fn easing_span(&self, index: usize) -> f64 {
        let value = |i: usize| self.keys.get(i).and_then(scalar);
        let span = |i: usize| match (value(i), value(i + 1)) {
            (Some(a), Some(b)) => (b - a).abs(),
            _ => 0.0,
        };
        let own = span(index);
        if own > LINEAR_EPSILON {
            return own;
        }
        (0..index)
            .rev()
            .chain(index + 1..self.keys.len().saturating_sub(1))
            .map(span)
            .find(|span| *span > LINEAR_EPSILON)
            .unwrap_or(FALLBACK_SPAN)
    }

    /// The segment leaving the key at `time` as a normalized curve, or `None`
    /// for the last key, a non-scalar channel or a held segment.
    pub fn segment_curve(&self, time: Tick) -> Option<EasingCurve> {
        let index = self.keys.binary_search_by_key(&time, |key| key.time).ok()?;
        let a = self.keys.get(index)?;
        let b = self.keys.get(index + 1)?;
        let (av, bv) = (scalar(a)?, scalar(b)?);
        match a.segment {
            Segment::Step => return None,
            Segment::Linear => return Some(EasingCurve::LINEAR),
            Segment::Bezier => {}
        }
        let span = ticks(b.time, a.time);
        let scale = self.easing_span(index);
        let delta = if bv == av { scale } else { bv - av };
        let out = a.outgoing.map_or((1.0 / 3.0, delta / 3.0), |h| {
            ((h.dt.0 as f64 / span).clamp(0.0, 1.0), h.dv)
        });
        let incoming = b.incoming.map_or((-1.0 / 3.0, -delta / 3.0), |h| {
            ((h.dt.0 as f64 / span).clamp(-1.0, 0.0), h.dv)
        });
        Some(EasingCurve([
            out.0,
            out.1 / delta,
            1.0 + incoming.0,
            1.0 + incoming.1 / delta,
        ]))
    }

    /// Give the segment leaving the key at `time` the normalized `curve`: both
    /// handles, the segment kind and both keys' tangent labels change together,
    /// or nothing does. The straight line stores no handles.
    pub fn set_easing(&mut self, time: Tick, curve: EasingCurve) -> Result<()> {
        curve.validate()?;
        let index = self
            .keys
            .binary_search_by_key(&time, |key| key.time)
            .map_err(|_| Error::Invalid("missing key"))?;
        if index + 1 >= self.keys.len() {
            return Err(Error::Invalid("no outgoing segment"));
        }
        let scale = self.easing_span(index);
        let mut next = self.clone();
        let (head, tail) = next.keys.split_at_mut(index + 1);
        let (Some(a), Some(b)) = (head.last_mut(), tail.first_mut()) else {
            return Err(Error::Invalid("no outgoing segment"));
        };
        let (Some(av), Some(bv)) = (scalar(a), scalar(b)) else {
            return Err(Error::Invalid("easing requires scalar keys"));
        };
        if curve.is_linear() {
            a.segment = Segment::Linear;
            a.outgoing = None;
            b.incoming = None;
        } else {
            let [x1, y1, x2, y2] = curve.0;
            let span = ticks(b.time, a.time);
            let delta = if bv == av { scale } else { bv - av };
            a.segment = Segment::Bezier;
            a.outgoing = Some(CurveHandle {
                dt: Tick((x1 * span).round() as i64),
                dv: y1 * delta,
            });
            b.incoming = Some(CurveHandle {
                dt: Tick(((x2 - 1.0) * span).round() as i64),
                dv: (y2 - 1.0) * delta,
            });
        }
        relabel(a);
        relabel(b);
        next.validate()?;
        *self = next;
        Ok(())
    }
}
