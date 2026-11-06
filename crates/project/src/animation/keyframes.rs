//! Keyed scalar channels and their exact evaluation.
use super::params::{MAX_VALUE, ParamValue};
use crate::{Error, Result};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, SeqAccess, Visitor},
};
use std::fmt;
use time::Tick;

pub const MAX_CHANNEL_KEYS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Segment {
    Step,
    Linear,
    Bezier,
}
/// Persisted source labels only; they do not synthesize or modify handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TangentMode {
    Auto,
    Aligned,
    Broken,
    Flat,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Extrapolation {
    Hold,
    Linear,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HandleSide {
    In,
    Out,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CurveHandle {
    /// Signed relative ticks: incoming <= 0, outgoing >= 0; clamped to segment span at evaluation.
    pub dt: Tick,
    pub dv: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Keyframe {
    /// Relative to item start; negative/out-of-window keys retain trimmed curves.
    pub time: Tick,
    pub value: ParamValue,
    pub segment: Segment,
    pub tangent: TangentMode,
    pub incoming: Option<CurveHandle>,
    pub outgoing: Option<CurveHandle>,
}
impl Keyframe {
    pub fn new(time: Tick, value: ParamValue) -> Self {
        Self {
            time,
            value,
            segment: Segment::Linear,
            tangent: TangentMode::Flat,
            incoming: None,
            outgoing: None,
        }
    }
    fn validate(&self, base: &ParamValue) -> Result<()> {
        self.value.validate()?;
        if !self.value.same_kind(base) {
            return Err(Error::Invalid("mixed channel values"));
        }
        for (handle, incoming) in [(self.incoming, true), (self.outgoing, false)] {
            if let Some(h) = handle
                && (!matches!(base, ParamValue::Scalar(_))
                    || !h.dv.is_finite()
                    || h.dv.abs() > MAX_VALUE
                    || (incoming && h.dt > Tick::ZERO)
                    || (!incoming && h.dt < Tick::ZERO))
            {
                return Err(Error::Invalid("curve handle"));
            }
        }
        if !matches!(base, ParamValue::Scalar(_)) && self.segment != Segment::Step {
            return Err(Error::Invalid("discrete channel must hold"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "ChannelData")]
pub struct Channel {
    pub(super) value: ParamValue,
    pub(super) keys: Vec<Keyframe>,
    before: Extrapolation,
    after: Extrapolation,
}
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ChannelData {
    value: ParamValue,
    #[serde(deserialize_with = "keys")]
    keys: Vec<Keyframe>,
    before: Extrapolation,
    after: Extrapolation,
}
fn keys<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Vec<Keyframe>, D::Error> {
    struct Keys;
    impl<'de> Visitor<'de> for Keys {
        type Value = Vec<Keyframe>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("bounded keyframes")
        }
        fn visit_seq<A: SeqAccess<'de>>(
            self,
            mut a: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut keys = Vec::new();
            while let Some(key) = a.next_element()? {
                if keys.len() == MAX_CHANNEL_KEYS {
                    return Err(de::Error::custom("keyframe limit"));
                }
                keys.push(key);
            }
            Ok(keys)
        }
    }
    d.deserialize_seq(Keys)
}
impl TryFrom<ChannelData> for Channel {
    type Error = Error;
    fn try_from(d: ChannelData) -> Result<Self> {
        Self::new(d.value, d.keys, d.before, d.after)
    }
}
impl Channel {
    pub fn new(
        value: ParamValue,
        keys: Vec<Keyframe>,
        before: Extrapolation,
        after: Extrapolation,
    ) -> Result<Self> {
        let channel = Self {
            value,
            keys,
            before,
            after,
        };
        channel.validate()?;
        Ok(channel)
    }
    pub(super) fn validate(&self) -> Result<()> {
        self.value.validate()?;
        if matches!(self.value, ParamValue::Color(_)) {
            return Err(Error::Invalid("colour requires composite channels"));
        }
        if self.keys.len() > MAX_CHANNEL_KEYS {
            return Err(Error::TooLarge("channel keys"));
        }
        if !matches!(self.value, ParamValue::Scalar(_))
            && (self.before != Extrapolation::Hold || self.after != Extrapolation::Hold)
        {
            return Err(Error::Invalid("discrete extrapolation must hold"));
        }
        let mut previous = None;
        for key in &self.keys {
            key.validate(&self.value)?;
            if previous.is_some_and(|time| time >= key.time) {
                return Err(Error::Invalid("duplicate or unordered keys"));
            }
            previous = Some(key.time);
        }
        Ok(())
    }
    pub fn keys(&self) -> &[Keyframe] {
        &self.keys
    }
    pub fn base(&self) -> &ParamValue {
        &self.value
    }
    pub fn extrapolation(&self) -> (Extrapolation, Extrapolation) {
        (self.before, self.after)
    }
    /// Replace the outside-the-keys policy while keeping every key and the base value.
    pub fn set_extrapolation(&mut self, before: Extrapolation, after: Extrapolation) -> Result<()> {
        let mut next = self.clone();
        next.before = before;
        next.after = after;
        next.validate()?;
        *self = next;
        Ok(())
    }
    /// Returns exact stored endpoint values; scalar interior handles use segment-relative f64 math.
    pub fn value_at(&self, time: Tick) -> ParamValue {
        let Some(first) = self.keys.first() else {
            return self.value.clone();
        };
        let Some(last) = self.keys.last() else {
            return self.value.clone();
        };
        if time < first.time {
            if !matches!(self.value, ParamValue::Scalar(_)) {
                return self.value.clone();
            }
            return self.edge(first, self.keys.get(1), time, self.before);
        }
        if time > last.time {
            return self.edge(
                last,
                self.keys
                    .len()
                    .checked_sub(2)
                    .and_then(|i| self.keys.get(i)),
                time,
                self.after,
            );
        }
        let index = self.keys.partition_point(|key| key.time <= time);
        let Some(a) = index.checked_sub(1).and_then(|i| self.keys.get(i)) else {
            return first.value.clone();
        };
        if time == a.time {
            return a.value.clone();
        }
        let Some(b) = self.keys.get(index) else {
            return a.value.clone();
        };
        let (ParamValue::Scalar(av), ParamValue::Scalar(bv)) = (&a.value, &b.value) else {
            return a.value.clone();
        };
        if a.segment == Segment::Step {
            return a.value.clone();
        }
        let span = distance(b.time, a.time);
        let x = distance(time, a.time) / span;
        if a.segment == Segment::Linear {
            return ParamValue::Scalar(lerp(*av, *bv, x));
        }
        let out = a.outgoing.map_or((1.0 / 3.0, (bv - av) / 3.0), |h| {
            ((h.dt.0 as f64 / span).clamp(0.0, 1.0), h.dv)
        });
        let incoming = b.incoming.map_or((-1.0 / 3.0, -(bv - av) / 3.0), |h| {
            ((h.dt.0 as f64 / span).clamp(-1.0, 0.0), h.dv)
        });
        let progress = super::bezier::progress_for_x(out.0, 1.0 + incoming.0, x);
        ParamValue::Scalar(super::bezier::cubic_value(
            *av,
            *av + out.1,
            *bv + incoming.1,
            *bv,
            progress,
        ))
    }
    fn edge(
        &self,
        edge: &Keyframe,
        neighbor: Option<&Keyframe>,
        time: Tick,
        mode: Extrapolation,
    ) -> ParamValue {
        if mode == Extrapolation::Linear
            && let Some(neighbor) = neighbor
            && let (ParamValue::Scalar(a), ParamValue::Scalar(b)) = (&edge.value, &neighbor.value)
        {
            return ParamValue::Scalar(lerp(
                *a,
                *b,
                distance(time, edge.time) / distance(neighbor.time, edge.time),
            ));
        }
        edge.value.clone()
    }
    pub fn put(&mut self, key: Keyframe) -> Result<()> {
        key.validate(&self.value)?;
        match self.keys.binary_search_by_key(&key.time, |key| key.time) {
            Ok(i) => {
                *self.keys.get_mut(i).ok_or(Error::Invalid("key index"))? = key;
            }
            Err(i) => {
                if self.keys.len() == MAX_CHANNEL_KEYS {
                    return Err(Error::TooLarge("channel keys"));
                }
                self.keys.insert(i, key);
            }
        }
        Ok(())
    }
    pub fn remove(&mut self, time: Tick) -> Result<()> {
        let i = self
            .keys
            .binary_search_by_key(&time, |key| key.time)
            .map_err(|_| Error::Invalid("missing key"))?;
        self.keys.remove(i);
        Ok(())
    }
    pub fn retime_key(&mut self, from: Tick, to: Tick) -> Result<()> {
        let mut next = self.clone();
        let i = next
            .keys
            .binary_search_by_key(&from, |key| key.time)
            .map_err(|_| Error::Invalid("missing key"))?;
        if from != to && next.keys.binary_search_by_key(&to, |key| key.time).is_ok() {
            return Err(Error::Invalid("key time collision"));
        }
        next.keys
            .get_mut(i)
            .ok_or(Error::Invalid("key index"))?
            .time = to;
        next.keys.sort_by_key(|key| key.time);
        *self = next;
        Ok(())
    }
    pub fn set_handle(
        &mut self,
        time: Tick,
        side: HandleSide,
        handle: Option<CurveHandle>,
        tangent: TangentMode,
    ) -> Result<()> {
        let mut next = self.clone();
        let key = next
            .keys
            .iter_mut()
            .find(|key| key.time == time)
            .ok_or(Error::Invalid("missing key"))?;
        match side {
            HandleSide::In => key.incoming = handle,
            HandleSide::Out => key.outgoing = handle,
        }
        key.tangent = tangent;
        next.validate()?;
        *self = next;
        Ok(())
    }
    pub(super) fn map_times(
        &mut self,
        mut map: impl FnMut(Tick) -> Result<Tick>,
        handles: bool,
    ) -> Result<()> {
        for key in &mut self.keys {
            key.time = map(key.time)?;
            if handles {
                for h in [&mut key.incoming, &mut key.outgoing].into_iter().flatten() {
                    h.dt = map(h.dt)?;
                }
            }
        }
        self.validate()
    }
}
fn distance(a: Tick, b: Tick) -> f64 {
    (i128::from(a.0) - i128::from(b.0)) as f64
}
fn lerp(a: f64, b: f64, x: f64) -> f64 {
    a + (b - a) * x
}
