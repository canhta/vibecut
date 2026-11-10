//! Persisted effect instances and the parameter model; evaluation and devices
//! belong to render/apps.
use crate::{AnimationPath, Color, Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub use super::effects_catalog::{
    CHROMA_KEY_COLOR_DEFAULT, CHROMA_KEY_INTENSITY_DEFAULT, CHROMA_KEY_MAX, CHROMA_KEY_MIN,
    GRADE_MAX, GRADE_MIN, effect_definitions,
};

/// Where an effect is offered. The effects library lists the library shelves;
/// `Inspector` effects belong to a clip inspector panel and are never listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectShelf {
    BlurDetail,
    Colour,
    Distort,
    GlowLight,
    Stylize,
    OverlayKey,
    Inspector,
}
impl EffectShelf {
    pub const fn in_library(self) -> bool {
        !matches!(self, Self::Inspector)
    }
}

/// Angles are degrees within one turn either way.
pub const ANGLE_MAX: f64 = 360.0;
/// Points are fractions of the frame, 0..=1 on each axis.
pub const POINT_MAX: f64 = 1.0;
/// A packed colour scalar is exactly one 24-bit sRGB value (`0xRRGGBB`).
pub const PACKED_COLOR_MAX: f64 = 0xFF_FFFF as f64;

/// One typed parameter value. The definition's kind decides which variant a
/// parameter holds; `Scalar` carries both float and angle kinds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum EffectValue {
    Scalar(f64),
    Choice(u32),
    Bool(bool),
    Color(Color),
    Point([f64; 2]),
}
impl EffectValue {
    pub const fn scalar(&self) -> Option<f64> {
        match self {
            Self::Scalar(v) => Some(*v),
            _ => None,
        }
    }
    pub const fn choice(&self) -> Option<u32> {
        match self {
            Self::Choice(v) => Some(*v),
            _ => None,
        }
    }
    pub const fn flag(&self) -> Option<bool> {
        match self {
            Self::Bool(v) => Some(*v),
            _ => None,
        }
    }
    pub const fn color(&self) -> Option<Color> {
        match self {
            Self::Color(v) => Some(*v),
            _ => None,
        }
    }
    pub const fn point(&self) -> Option<[f64; 2]> {
        match self {
            Self::Point(v) => Some(*v),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EffectParamKind {
    /// `min..=max` is the hard bound a typed value must satisfy; the soft
    /// range is what a slider spans.
    Float {
        min: f64,
        max: f64,
        soft_min: f64,
        soft_max: f64,
    },
    /// Degrees, `-ANGLE_MAX..=ANGLE_MAX`.
    Angle,
    /// An index into the named options.
    Choice {
        options: &'static [&'static str],
    },
    Bool,
    Color,
    /// A frame fraction on each axis, `0..=POINT_MAX`.
    Point,
}
#[derive(Clone, Copy, Debug)]
pub struct EffectParamDefinition {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: EffectParamKind,
    pub default: EffectValue,
    /// Scalar channels (float, angle) may carry keyframes.
    pub animatable: bool,
}
impl EffectParamDefinition {
    /// Hard bound of a scalar parameter; `None` for the other kinds.
    pub const fn scalar_range(&self) -> Option<(f64, f64)> {
        match self.kind {
            EffectParamKind::Float { min, max, .. } => Some((min, max)),
            EffectParamKind::Angle => Some((-ANGLE_MAX, ANGLE_MAX)),
            _ => None,
        }
    }
    /// Check `value` against this parameter's kind and bounds.
    pub fn check(&self, value: &EffectValue) -> Result<()> {
        let valid = match (self.kind, value) {
            (EffectParamKind::Float { .. } | EffectParamKind::Angle, EffectValue::Scalar(v)) => {
                self.scalar_range()
                    .is_some_and(|(min, max)| v.is_finite() && (min..=max).contains(v))
            }
            (EffectParamKind::Choice { options }, EffectValue::Choice(i)) => {
                (*i as usize) < options.len()
            }
            (EffectParamKind::Bool, EffectValue::Bool(_))
            | (EffectParamKind::Color, EffectValue::Color(_)) => true,
            (EffectParamKind::Point, EffectValue::Point(p)) => p
                .iter()
                .all(|v| v.is_finite() && (0.0..=POINT_MAX).contains(v)),
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(Error::Invalid("effect parameter value"))
        }
    }
}
/// UI/catalog metadata lives beside the typed persisted parameters.
#[derive(Clone, Copy, Debug)]
pub struct EffectDefinition {
    pub id: &'static str,
    pub name: &'static str,
    pub shelf: EffectShelf,
    pub params: &'static [EffectParamDefinition],
}
impl EffectDefinition {
    pub fn param(&self, id: &str) -> Option<&'static EffectParamDefinition> {
        self.params.iter().find(|param| param.id == id)
    }
    /// Ids of the parameters that may carry keyframes.
    pub fn animated_ids(&self) -> impl Iterator<Item = &'static str> + use<> {
        self.params
            .iter()
            .filter(|param| param.animatable)
            .map(|param| param.id)
    }
    /// Every parameter at its default.
    pub fn defaults(&self) -> BTreeMap<String, EffectValue> {
        self.params
            .iter()
            .map(|param| (param.id.to_owned(), param.default))
            .collect()
    }
    pub fn instance(&self, id: impl Into<String>) -> EffectInstance {
        EffectInstance {
            id: id.into(),
            kind: self.id.to_owned(),
            enabled: true,
            params: self.defaults(),
        }
    }
}
/// The definition of one effect kind.
pub fn effect_definition(kind: &str) -> Result<&'static EffectDefinition> {
    effect_definitions()
        .iter()
        .find(|definition| definition.id == kind)
        .ok_or(Error::Invalid("effect kind"))
}

/// Stable instance identity is local to an item and also names its animation paths.
/// `params` holds exactly the definition's parameters, keyed by id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectInstance {
    pub id: String,
    pub kind: String,
    pub enabled: bool,
    pub params: BTreeMap<String, EffectValue>,
}

/// One exact 24-bit sRGB colour packed into a scalar, opaque. A colour pick
/// arrives as one packed `0xRRGGBB` scalar so it is a single effect command.
pub fn packed_color(value: f64) -> Result<Color> {
    if !value.is_finite() || !(0.0..=PACKED_COLOR_MAX).contains(&value) || value.fract() != 0.0 {
        return Err(Error::Invalid("packed colour"));
    }
    let packed = value as u32;
    Ok(Color([
        (packed >> 16) as u8,
        (packed >> 8) as u8,
        packed as u8,
        255,
    ]))
}
impl EffectInstance {
    pub fn definition(&self) -> Result<&'static EffectDefinition> {
        effect_definition(&self.kind)
    }
    pub fn value(&self, id: &str) -> Option<&EffectValue> {
        self.params.get(id)
    }
    /// Animation path of one scalar parameter, mirroring the catalog key.
    pub fn param_path(&self, key: &str) -> Result<AnimationPath> {
        AnimationPath::new(format!("effects.{}.params.{key}", self.id))
    }

    /// Replace one parameter with a typed value checked against its definition.
    pub fn with(mut self, id: &str, value: EffectValue) -> Result<Self> {
        self.definition()?
            .param(id)
            .ok_or(Error::Unsupported("effect parameter"))?
            .check(&value)?;
        self.params.insert(id.to_owned(), value);
        Ok(self)
    }

    /// Change one registered parameter from a scalar, refusing unknown ids and
    /// invalid values atomically. Scalars and angles take the number itself, a
    /// choice its index, a bool 0 or 1 and a colour its packed `0xRRGGBB`;
    /// points need two components and are not addressable here.
    pub fn set_scalar(&mut self, id: &str, value: f64) -> Result<()> {
        let param = self
            .definition()?
            .param(id)
            .ok_or(Error::Unsupported("effect parameter"))?;
        let next = match param.kind {
            EffectParamKind::Float { .. } | EffectParamKind::Angle => EffectValue::Scalar(value),
            EffectParamKind::Choice { .. } => {
                if !value.is_finite()
                    || value.fract() != 0.0
                    || !(0.0..=f64::from(u32::MAX)).contains(&value)
                {
                    return Err(Error::Invalid("effect choice"));
                }
                EffectValue::Choice(value as u32)
            }
            EffectParamKind::Bool => match value {
                0.0 => EffectValue::Bool(false),
                1.0 => EffectValue::Bool(true),
                _ => return Err(Error::Invalid("effect flag")),
            },
            EffectParamKind::Color => EffectValue::Color(packed_color(value)?),
            EffectParamKind::Point => return Err(Error::Unsupported("effect parameter")),
        };
        param.check(&next)?;
        self.params.insert(id.to_owned(), next);
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        let definition = self.definition()?;
        if self.params.len() != definition.params.len() {
            return Err(Error::Invalid("effect parameter set"));
        }
        for param in definition.params {
            let value = self
                .params
                .get(param.id)
                .ok_or(Error::Invalid("effect parameter set"))?;
            param.check(value)?;
        }
        for id in definition.animated_ids() {
            self.param_path(id)?;
        }
        Ok(())
    }
}
