//! Units and editor bounds for the existing transform animation channels.
use crate::{AnimationPath, Animations, BuiltinPath, Error, ParamValue, Result};
use serde::{Deserialize, Serialize};
use time::Tick;

/// One reflected local axis. A flip mirrors the placed item about its centre
/// before clockwise rotation and keeps scale magnitudes positive.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum FlipAxis {
    Horizontal,
    Vertical,
}

impl FlipAxis {
    pub const ALL: [Self; 2] = [Self::Horizontal, Self::Vertical];
    pub const fn builtin(self) -> BuiltinPath {
        match self {
            Self::Horizontal => BuiltinPath::TransformFlipX,
            Self::Vertical => BuiltinPath::TransformFlipY,
        }
    }
    /// The held Boolean at real item-local time; an absent channel is not flipped.
    pub fn value_at(self, animations: &Animations, at: Tick) -> Result<bool> {
        match animations.value_at(&AnimationPath::from(self.builtin()), at)? {
            None => Ok(false),
            Some(ParamValue::Bool(value)) => Ok(value),
            _ => Err(Error::Invalid("flip value")),
        }
    }
}

/// Canvas edge or centre a selected item's transformed box is moved to.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ItemAlignment {
    Left,
    HorizontalCenter,
    Right,
    Top,
    VerticalCenter,
    Bottom,
}

/// Axis along which three or more selected items are spaced evenly.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DistributionAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransformField {
    PositionX,
    PositionY,
    ScaleX,
    ScaleY,
    Rotate,
}

impl TransformField {
    pub const ALL: [Self; 5] = [
        Self::PositionX,
        Self::PositionY,
        Self::ScaleX,
        Self::ScaleY,
        Self::Rotate,
    ];
    pub const fn builtin(self) -> BuiltinPath {
        match self {
            Self::PositionX => BuiltinPath::TransformPositionX,
            Self::PositionY => BuiltinPath::TransformPositionY,
            Self::ScaleX => BuiltinPath::TransformScaleX,
            Self::ScaleY => BuiltinPath::TransformScaleY,
            Self::Rotate => BuiltinPath::TransformRotate,
        }
    }
    /// Position is in project canvas pixels, scale is a ratio, rotation is degrees clockwise.
    pub const fn bounds(self) -> (f64, f64) {
        match self {
            Self::PositionX | Self::PositionY => (-100_000.0, 100_000.0),
            Self::ScaleX | Self::ScaleY => (0.01, 10.0),
            Self::Rotate => (-360.0, 360.0),
        }
    }
    pub const fn default_value(self) -> f64 {
        match self {
            Self::ScaleX | Self::ScaleY => 1.0,
            _ => 0.0,
        }
    }
    /// Refuses non-finite input; clamps finite editor/animated values to the bounds.
    pub fn clamp(self, value: f64) -> Result<f64> {
        if !value.is_finite() {
            return Err(Error::Invalid("transform value"));
        }
        let (min, max) = self.bounds();
        Ok(value.clamp(min, max))
    }
    pub fn value_at(self, animations: &Animations, at: Tick) -> Result<f64> {
        match animations.value_at(&AnimationPath::from(self.builtin()), at)? {
            None => Ok(self.default_value()),
            Some(ParamValue::Scalar(value)) => self.clamp(value),
            _ => Err(Error::Invalid("transform scalar")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transform_bounds_defaults_and_nonfinite_values() {
        for field in TransformField::ALL {
            let (min, max) = field.bounds();
            assert_eq!(field.clamp(min - 1.0).ok(), Some(min));
            assert_eq!(field.clamp(max + 1.0).ok(), Some(max));
            for value in [min, max, field.default_value()] {
                assert_eq!(field.clamp(value).ok(), Some(value));
            }
            assert!(field.clamp(f64::NAN).is_err());
            assert!(field.clamp(f64::INFINITY).is_err());
            assert_eq!(
                field.value_at(&Animations::default(), Tick::ZERO).ok(),
                Some(field.default_value())
            );
        }
    }
}
