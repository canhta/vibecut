//! Source-normalized crop rectangle carried by video and image items.

use serde::{Deserialize, Serialize};

/// Region of the source frame kept when an item is drawn. `x` and `y` are the
/// top-left corner, and `width`/`height` the size, as fractions of the source
/// frame (0..=1); the rectangle is nonempty, finite and contained in the frame.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Crop {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
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
}
