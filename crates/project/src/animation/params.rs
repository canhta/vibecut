//! Animation paths, typed parameter values and the per-item animation map.
use super::keyframes::{Channel, Extrapolation};
use crate::{Error, Result};
use captions::NfcText;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, MapAccess, Visitor},
};
use std::{collections::BTreeMap, fmt};
use time::{Rounding, Tick};

pub const MAX_ANIMATION_PATHS: usize = 64;
pub const MAX_PROJECT_KEYS: usize = 100_000;
pub(super) const MAX_VALUE: f64 = 1.0e12;

/// Built-in animation channels: the fixed vocabulary persisted without an
/// extension id. [`Self::as_str`] is the stored spelling and never changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BuiltinPath {
    TransformPositionX,
    TransformPositionY,
    TransformScaleX,
    TransformScaleY,
    TransformRotate,
    TransformFlipX,
    TransformFlipY,
    Opacity,
    Volume,
    Muted,
    Color,
    StrokeColor,
    BackgroundColor,
    BackgroundPaddingX,
    BackgroundPaddingY,
    BackgroundOffsetX,
    BackgroundOffsetY,
    BackgroundCornerRadius,
    CropX,
    CropY,
    CropAuto,
}
impl BuiltinPath {
    pub const ALL: [Self; 21] = [
        Self::TransformPositionX,
        Self::TransformPositionY,
        Self::TransformScaleX,
        Self::TransformScaleY,
        Self::TransformRotate,
        Self::TransformFlipX,
        Self::TransformFlipY,
        Self::Opacity,
        Self::Volume,
        Self::Muted,
        Self::Color,
        Self::StrokeColor,
        Self::BackgroundColor,
        Self::BackgroundPaddingX,
        Self::BackgroundPaddingY,
        Self::BackgroundOffsetX,
        Self::BackgroundOffsetY,
        Self::BackgroundCornerRadius,
        Self::CropX,
        Self::CropY,
        Self::CropAuto,
    ];
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TransformPositionX => "transform.positionX",
            Self::TransformPositionY => "transform.positionY",
            Self::TransformScaleX => "transform.scaleX",
            Self::TransformScaleY => "transform.scaleY",
            Self::TransformRotate => "transform.rotate",
            Self::TransformFlipX => "transform.flipX",
            Self::TransformFlipY => "transform.flipY",
            Self::Opacity => "opacity",
            Self::Volume => "volume",
            Self::Muted => "muted",
            Self::Color => "color",
            Self::StrokeColor => "stroke.color",
            Self::BackgroundColor => "background.color",
            Self::BackgroundPaddingX => "background.paddingX",
            Self::BackgroundPaddingY => "background.paddingY",
            Self::BackgroundOffsetX => "background.offsetX",
            Self::BackgroundOffsetY => "background.offsetY",
            Self::BackgroundCornerRadius => "background.cornerRadius",
            Self::CropX => "crop.x",
            Self::CropY => "crop.y",
            Self::CropAuto => "crop.auto",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|path| path.as_str() == value)
    }
    /// Reflection channels hold a step Boolean; they are never numeric scale signs.
    pub const fn is_flip(self) -> bool {
        matches!(self, Self::TransformFlipX | Self::TransformFlipY)
    }
}

/// Item-local extension identifier grammar shared with persisted effect instances.
pub(crate) fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

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
impl From<BuiltinPath> for AnimationPath {
    fn from(value: BuiltinPath) -> Self {
        Self(value.as_str().to_owned())
    }
}
impl AnimationPath {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.len() > 160 {
            return Err(Error::Invalid("animation path"));
        }
        if BuiltinPath::parse(&value).is_some() {
            return Ok(Self(value));
        }
        let parts: Vec<_> = value.split('.').collect();
        let extension = match parts.as_slice() {
            ["params", name] => identifier(name),
            ["effects" | "masks", id, "params", name] => identifier(id) && identifier(name),
            _ => false,
        };
        if !extension {
            return Err(Error::Invalid("animation path"));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// The typed built-in channel, or `None` for an extension path.
    pub fn builtin(&self) -> Option<BuiltinPath> {
        BuiltinPath::parse(&self.0)
    }
    /// Independent scalar channels sharing an editor group, never forced equal.
    pub fn scale_group() -> [BuiltinPath; 2] {
        [BuiltinPath::TransformScaleX, BuiltinPath::TransformScaleY]
    }
    fn accepts(&self, param: &Param) -> bool {
        let Some(builtin) = self.builtin() else {
            return true;
        };
        let base = match param {
            Param::Value(c) => Some(c.base()),
            Param::Color(_) => None,
        };
        match builtin {
            BuiltinPath::Muted
            | BuiltinPath::CropAuto
            | BuiltinPath::TransformFlipX
            | BuiltinPath::TransformFlipY => matches!(base, Some(ParamValue::Bool(_))),
            BuiltinPath::Color | BuiltinPath::StrokeColor | BuiltinPath::BackgroundColor => {
                matches!(param, Param::Color(_)) || matches!(base, Some(ParamValue::Text(_)))
            }
            _ => matches!(base, Some(ParamValue::Scalar(_))),
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
    pub(super) fn validate(&self) -> Result<()> {
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
    pub(super) fn same_kind(&self, other: &Self) -> bool {
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
    pub fn remove(&mut self, path: &AnimationPath) -> Option<Param> {
        self.0.remove(path)
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
