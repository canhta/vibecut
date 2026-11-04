//! Validated item-relative channels. Rendering and editing share this evaluator.
use crate::{Error, Result};
use captions::NfcText;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use std::{collections::BTreeMap, fmt};
use time::{Rounding, Tick};

pub const MAX_CHANNEL_KEYS: usize = 4096;
pub const MAX_ANIMATION_PATHS: usize = 64;
pub const MAX_PROJECT_KEYS: usize = 100_000;
const MAX_VALUE: f64 = 1.0e12;

#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(try_from = "String")]
pub struct AnimationPath(String);
impl TryFrom<String> for AnimationPath {
    type Error = Error;
    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}
impl AnimationPath {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.len() > 160 {
            return Err(Error::Invalid("animation path"));
        }
        let builtin = matches!(
            value.as_str(),
            "transform.positionX"
                | "transform.positionY"
                | "transform.scaleX"
                | "transform.scaleY"
                | "transform.rotate"
                | "opacity"
                | "volume"
                | "color"
                | "background.color"
                | "background.paddingX"
                | "background.paddingY"
                | "background.offsetX"
                | "background.offsetY"
                | "background.cornerRadius"
        );
        fn identifier(s: &str) -> bool {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        }
        let parts: Vec<_> = value.split('.').collect();
        let extension = match parts.as_slice() {
            ["params", name] => identifier(name),
            ["effects", id, "params", name] => identifier(id) && identifier(name),
            _ => false,
        };
        if value.len() > 160 || (!builtin && !extension) {
            return Err(Error::Invalid("animation path"));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// Independent scalar channels sharing an editor group, never forced equal.
    pub fn scale_group() -> [&'static str; 2] {
        ["transform.scaleX", "transform.scaleY"]
    }
    fn accepts(&self, param: &Param) -> bool {
        if self.0.starts_with("params.") || self.0.starts_with("effects.") {
            return true;
        }
        let base = match param {
            Param::Value(c) => Some(c.base()),
            Param::Color(_) => None,
        };
        if matches!(self.0.as_str(), "color" | "background.color") {
            matches!(param, Param::Color(_)) || matches!(base, Some(ParamValue::Text(_)))
        } else {
            matches!(base, Some(ParamValue::Scalar(_)))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ParamValue {
    Scalar(f64),
    Bool(bool),
    Text(NfcText),
    /// Linear-light, unpremultiplied RGBA. Components evaluate independently.
    Color([f64; 4]),
}
impl ParamValue {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Scalar(v) if !v.is_finite() || v.abs() > MAX_VALUE => {
                Err(Error::Invalid("animation value"))
            }
            Self::Color(c) if c.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) => {
                Err(Error::Invalid("linear RGBA value"))
            }
            Self::Text(t) if t.as_str().len() > 1 << 20 => Err(Error::TooLarge("animation text")),
            _ => Ok(()),
        }
    }
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
    fn scalar(&self) -> Result<f64> {
        if let Self::Scalar(v) = self {
            Ok(*v)
        } else {
            Err(Error::Invalid("scalar channel required"))
        }
    }
}
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
    value: ParamValue,
    keys: Vec<Keyframe>,
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
    fn validate(&self) -> Result<()> {
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
        let progress = crate::bezier::progress_for_x(out.0, 1.0 + incoming.0, x);
        ParamValue::Scalar(crate::bezier::cubic_value(
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
    fn map_times(
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ColorComponent {
    R,
    G,
    B,
    A,
}
impl ColorComponent {
    fn index(self) -> usize {
        match self {
            Self::R => 0,
            Self::G => 1,
            Self::B => 2,
            Self::A => 3,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(
    tag = "kind",
    content = "channels",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[serde(try_from = "ParamData")]
pub enum Param {
    Value(Channel),
    Color(Box<[Channel; 4]>),
}
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(
    tag = "kind",
    content = "channels",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum ParamData {
    Value(Channel),
    Color(Box<[Channel; 4]>),
}
impl TryFrom<ParamData> for Param {
    type Error = Error;
    fn try_from(value: ParamData) -> Result<Self> {
        let value = match value {
            ParamData::Value(c) => Self::Value(c),
            ParamData::Color(c) => Self::Color(c),
        };
        value.validate()?;
        Ok(value)
    }
}
impl Param {
    pub fn new(value: ParamValue) -> Result<Self> {
        value.validate()?;
        let channel = |v| Channel::new(v, Vec::new(), Extrapolation::Hold, Extrapolation::Hold);
        match value {
            ParamValue::Color([r, g, b, a]) => Ok(Self::Color(Box::new([
                channel(ParamValue::Scalar(r))?,
                channel(ParamValue::Scalar(g))?,
                channel(ParamValue::Scalar(b))?,
                channel(ParamValue::Scalar(a))?,
            ]))),
            v => Ok(Self::Value(channel(v)?)),
        }
    }
    pub fn value_at(&self, time: Tick) -> Result<ParamValue> {
        match self {
            Self::Value(channel) => Ok(channel.value_at(time)),
            Self::Color(channels) => {
                let mut rgba = [0.0; 4];
                for (out, channel) in rgba.iter_mut().zip(channels.iter()) {
                    *out = channel.value_at(time).scalar()?.clamp(0.0, 1.0);
                }
                Ok(ParamValue::Color(rgba))
            }
        }
    }
    pub fn channel(&self, component: Option<ColorComponent>) -> Result<&Channel> {
        match (self, component) {
            (Self::Value(c), None) => Ok(c),
            (Self::Color(c), Some(i)) => c.get(i.index()).ok_or(Error::Invalid("RGBA component")),
            _ => Err(Error::Invalid("animation component")),
        }
    }
    pub fn channel_mut(&mut self, component: Option<ColorComponent>) -> Result<&mut Channel> {
        match (self, component) {
            (Self::Value(c), None) => Ok(c),
            (Self::Color(c), Some(i)) => {
                c.get_mut(i.index()).ok_or(Error::Invalid("RGBA component"))
            }
            _ => Err(Error::Invalid("animation component")),
        }
    }
    fn channels(&self) -> &[Channel] {
        match self {
            Self::Value(c) => std::slice::from_ref(c),
            Self::Color(c) => c.as_ref(),
        }
    }
    fn channels_mut(&mut self) -> &mut [Channel] {
        match self {
            Self::Value(c) => std::slice::from_mut(c),
            Self::Color(c) => c.as_mut(),
        }
    }
    fn validate(&self) -> Result<()> {
        for channel in self.channels() {
            channel.validate()?;
            if matches!(self, Self::Color(_)) {
                for value in
                    std::iter::once(&channel.value).chain(channel.keys.iter().map(|key| &key.value))
                {
                    if !(0.0..=1.0).contains(&value.scalar()?) {
                        return Err(Error::Invalid("linear RGBA channel"));
                    }
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct Animations(BTreeMap<AnimationPath, Param>);
impl<'de> Deserialize<'de> for Animations {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct Map;
        impl<'de> Visitor<'de> for Map {
            type Value = Animations;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("bounded animation paths")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut result = Animations::default();
                while let Some((path, param)) = a.next_entry()? {
                    if result.0.contains_key(&path) {
                        return Err(de::Error::custom("duplicate animation path"));
                    }
                    result.set(path, param).map_err(de::Error::custom)?;
                }
                Ok(result)
            }
        }
        d.deserialize_map(Map)
    }
}
impl Animations {
    pub fn iter(&self) -> impl Iterator<Item = (&AnimationPath, &Param)> {
        self.0.iter()
    }
    pub fn get(&self, path: &AnimationPath) -> Option<&Param> {
        self.0.get(path)
    }
    pub fn get_mut(&mut self, path: &AnimationPath) -> Result<&mut Param> {
        self.0
            .get_mut(path)
            .ok_or(Error::Invalid("missing animated parameter"))
    }
    pub fn set(&mut self, path: AnimationPath, param: Param) -> Result<()> {
        param.validate()?;
        if !path.accepts(&param) {
            return Err(Error::Invalid("animation path value type"));
        }
        if self.0.len() == MAX_ANIMATION_PATHS && !self.0.contains_key(&path) {
            return Err(Error::TooLarge("animation paths"));
        }
        let removed = self.0.get(&path).map_or(0, |p| {
            p.channels().iter().map(|c| c.keys.len()).sum::<usize>()
        });
        let added = param.channels().iter().map(|c| c.keys.len()).sum::<usize>();
        if self.key_count() - removed + added > MAX_PROJECT_KEYS {
            return Err(Error::TooLarge("animation keys"));
        }
        self.0.insert(path, param);
        Ok(())
    }
    pub fn value_at(&self, path: &AnimationPath, time: Tick) -> Result<Option<ParamValue>> {
        self.get(path).map(|p| p.value_at(time)).transpose()
    }
    pub fn key_count(&self) -> usize {
        self.0
            .values()
            .flat_map(Param::channels)
            .map(|c| c.keys.len())
            .sum()
    }
    pub(crate) fn validate(&self) -> Result<()> {
        if self.key_count() > MAX_PROJECT_KEYS {
            return Err(Error::TooLarge("animation keys"));
        }
        if self.0.len() > MAX_ANIMATION_PATHS {
            return Err(Error::TooLarge("animation paths"));
        }
        for (path, param) in &self.0 {
            param.validate()?;
            if !path.accepts(param) {
                return Err(Error::Invalid("animation path value type"));
            }
        }
        Ok(())
    }
    /// Translate all keys, retaining out-of-window anchors so trims/splits preserve exact curves.
    pub fn shift(&mut self, delta: Tick) -> Result<()> {
        if delta == Tick::ZERO {
            return self.validate();
        }
        self.map_times(
            |t| {
                t.checked_add(delta)
                    .ok_or(Error::Invalid("animation time overflow"))
            },
            false,
        )
    }
    /// Positive rational time stretch; nearest tick, ties away from zero. Collisions refuse atomically.
    /// Playback rate/media duration remain the retime consumer's responsibility.
    pub fn retime(&mut self, num: i64, den: i64) -> Result<()> {
        Tick::ZERO.checked_scale(num, den, Rounding::Nearest)?;
        self.map_times(|t| Ok(t.checked_scale(num, den, Rounding::Nearest)?), true)
    }
    fn map_times(
        &mut self,
        mut map: impl FnMut(Tick) -> Result<Tick>,
        handles: bool,
    ) -> Result<()> {
        let mut next = self.clone();
        for param in next.0.values_mut() {
            for channel in param.channels_mut() {
                channel.map_times(&mut map, handles)?;
            }
        }
        next.validate()?;
        *self = next;
        Ok(())
    }
}
