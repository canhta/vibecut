//! Source-normalized crop rectangle carried by video and image items.

use crate::{AnimationPath, Animations, BuiltinPath, Error, Param, ParamValue, Result};
use serde::{Deserialize, Serialize};

/// Region of the source frame kept when an item is drawn. `x` and `y` are the
/// top-left corner, and `width`/`height` the size, as fractions of the source
/// frame (0..=1); the rectangle is nonempty, finite and contained in the frame.
/// The default is the full frame.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Crop {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Default for Crop {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    }
}

impl Crop {
    /// Whether every field is finite, the size is positive and the rectangle
    /// stays inside the source frame.
    pub fn valid(&self) -> bool {
        let finite = |v: f64| v.is_finite();
        finite(self.x)
            && finite(self.y)
            && finite(self.width)
            && finite(self.height)
            && self.width > 0.0
            && self.height > 0.0
            && self.x >= 0.0
            && self.y >= 0.0
            && self.x + self.width <= 1.0
            && self.y + self.height <= 1.0
    }

    /// The largest top-left position `x` / `y` that keeps the rectangle inside
    /// the frame. Animated positions are bounded by `[0, max]`.
    pub fn max_position(&self) -> (f64, f64) {
        (1.0 - self.width, 1.0 - self.height)
    }

    /// The top-left corner with the animated `crop.x` / `crop.y` channels
    /// evaluated at `local` and clamped into the frame. Preview and export call
    /// this one function. Width and height stay static.
    pub fn at(&self, animations: &Animations, local: time::Tick) -> Result<Self> {
        let (max_x, max_y) = self.max_position();
        let channel = |path: BuiltinPath, fallback: f64, max: f64| match animations
            .value_at(&AnimationPath::from(path), local)?
        {
            None => Ok(fallback),
            Some(ParamValue::Scalar(v)) => Ok(v.clamp(0.0, max)),
            Some(_) => Err(Error::Invalid("crop scalar")),
        };
        Ok(Self {
            x: channel(BuiltinPath::CropX, self.x, max_x)?,
            y: channel(BuiltinPath::CropY, self.y, max_y)?,
            ..*self
        })
    }

    /// Checks the three crop channels of one item: they need a static crop,
    /// and every stored position stays within the frame bound.
    pub(crate) fn validate_animations(crop: Option<&Self>, animations: &Animations) -> Result<()> {
        for path in [
            BuiltinPath::CropX,
            BuiltinPath::CropY,
            BuiltinPath::CropAuto,
        ] {
            let Some(param) = animations.get(&AnimationPath::from(path)) else {
                continue;
            };
            let Some(crop) = crop else {
                return Err(Error::Invalid("crop animation without a static crop"));
            };
            let max = match path {
                BuiltinPath::CropX => crop.max_position().0,
                BuiltinPath::CropY => crop.max_position().1,
                _ => continue,
            };
            let Param::Value(channel) = param else {
                return Err(Error::Invalid("crop animation must be scalar"));
            };
            let values =
                std::iter::once(channel.base()).chain(channel.keys().iter().map(|k| &k.value));
            for value in values {
                match value {
                    ParamValue::Scalar(v) if (0.0..=max).contains(v) => {}
                    _ => return Err(Error::Invalid("crop position outside the frame")),
                }
            }
        }
        Ok(())
    }
}
