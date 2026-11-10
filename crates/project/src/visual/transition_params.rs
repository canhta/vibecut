//! Typed transition parameters: definitions owned by the catalogue, values persisted per transition.
use crate::{Color, Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const SEED_MAX: u32 = 9999;
const ANGLE_MAX: f64 = 1440.0;

/// Persisted overrides keyed by parameter id; an absent id uses the catalogue default.
pub type TransitionParams = BTreeMap<String, TransitionValue>;

/// What a parameter holds, with its bounds and default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransitionParamKind {
    /// Finite number in `min..=max`; `unit` names it for display ("%", "px", "").
    Float {
        min: f64,
        max: f64,
        default: f64,
        unit: &'static str,
    },
    /// Index into `options`.
    Choice {
        options: &'static [&'static str],
        default: u32,
    },
    Bool {
        default: bool,
    },
    /// Straight-alpha sRGB RGBA8.
    Color {
        default: Color,
    },
    /// Fractions of the frame (0..=1 on each axis), origin top-left.
    Point {
        default: [f64; 2],
    },
    /// Degrees, clockwise, at most four turns either way.
    Angle {
        default: f64,
    },
    /// Integer in 0..=9999 driving procedural noise.
    Seed {
        default: u32,
    },
}

/// One parameter of a catalogue entry. Parameters are fixed per transition: they never animate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionParamDef {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: TransitionParamKind,
}

/// A persisted parameter value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum TransitionValue {
    Float(f64),
    Choice(u32),
    Bool(bool),
    Color(Color),
    Point([f64; 2]),
    Angle(f64),
    Seed(u32),
}

impl TransitionParamKind {
    pub fn default_value(&self) -> TransitionValue {
        match *self {
            Self::Float { default, .. } => TransitionValue::Float(default),
            Self::Choice { default, .. } => TransitionValue::Choice(default),
            Self::Bool { default } => TransitionValue::Bool(default),
            Self::Color { default } => TransitionValue::Color(default),
            Self::Point { default } => TransitionValue::Point(default),
            Self::Angle { default } => TransitionValue::Angle(default),
            Self::Seed { default } => TransitionValue::Seed(default),
        }
    }
    /// Refuse a value of another type, a non-finite number or one outside the bounds.
    pub fn validate(&self, value: &TransitionValue) -> Result<()> {
        let ok = match (self, value) {
            (Self::Float { min, max, .. }, TransitionValue::Float(v)) => {
                v.is_finite() && (*min..=*max).contains(v)
            }
            (Self::Choice { options, .. }, TransitionValue::Choice(v)) => {
                usize::try_from(*v).is_ok_and(|n| n < options.len())
            }
            (Self::Bool { .. }, TransitionValue::Bool(_))
            | (Self::Color { .. }, TransitionValue::Color(_)) => true,
            (Self::Point { .. }, TransitionValue::Point(v)) => {
                v.iter().all(|c| c.is_finite() && (0.0..=1.0).contains(c))
            }
            (Self::Angle { .. }, TransitionValue::Angle(v)) => {
                v.is_finite() && v.abs() <= ANGLE_MAX
            }
            (Self::Seed { .. }, TransitionValue::Seed(v)) => *v <= SEED_MAX,
            _ => false,
        };
        ok.then_some(())
            .ok_or(Error::Invalid("transition parameter value"))
    }
}

impl TransitionParamDef {
    /// Whether the definition itself is coherent: its default passes its own bounds.
    pub fn is_coherent(&self) -> bool {
        let bounds = match self.kind {
            TransitionParamKind::Float { min, max, .. } => min.is_finite() && min <= max,
            TransitionParamKind::Choice { options, .. } => !options.is_empty(),
            _ => true,
        };
        bounds && self.kind.validate(&self.kind.default_value()).is_ok()
    }
}
