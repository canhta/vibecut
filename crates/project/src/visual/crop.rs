//! Source crop region, placed on the canvas by a fit policy.

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

/// How a video or image item's cropped source is placed on the canvas. The
/// policy is evaluated at render time from the canvas and the source size, so a
/// canvas resize re-fits the item; it is never baked into the transform.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum FitMode {
    /// Whole source inside the canvas, aspect kept, centred (letterbox).
    Fit,
    /// Source covers the canvas, aspect kept, the overflow cropped.
    #[default]
    Fill,
    /// Source mapped onto the whole canvas, aspect ignored.
    Stretch,
    /// Source at its own pixel size, centred.
    None,
    /// Stretch base: the item's transform alone places it. A transform edit
    /// enters it, and choosing it first records the computed fit into the
    /// transform scale.
    Manual,
}

/// Placement of an item's cropped source on the canvas, in canvas fractions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FitBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// The renderer center-crops the source to the box instead of stretching it.
    pub cover: bool,
}

impl FitMode {
    /// Every mode in display order.
    pub const ALL: [Self; 5] = [
        Self::Fit,
        Self::Fill,
        Self::Stretch,
        Self::None,
        Self::Manual,
    ];

    /// The persisted spelling, also the command and control value.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Fit => "fit",
            Self::Fill => "fill",
            Self::Stretch => "stretch",
            Self::None => "none",
            Self::Manual => "manual",
        }
    }

    /// Normalized size of the whole cropped source after the policy, before any
    /// crop to the canvas: a Fill source keeps its overflow. `canvas` and
    /// `source` are pixel sizes; `None` for a non-positive or non-finite size.
    pub fn extent(self, canvas: [f64; 2], source: [f64; 2]) -> Option<[f64; 2]> {
        if canvas
            .iter()
            .chain(&source)
            .any(|v| !v.is_finite() || *v <= 0.0)
        {
            return None;
        }
        let scale = match self {
            Self::Fit => (canvas[0] / source[0]).min(canvas[1] / source[1]),
            Self::Fill => (canvas[0] / source[0]).max(canvas[1] / source[1]),
            Self::None => 1.0,
            Self::Stretch | Self::Manual => return Some([1.0, 1.0]),
        };
        Some([source[0] * scale / canvas[0], source[1] * scale / canvas[1]])
    }

    /// The one placement computation: render, handles, arrangement and canvas
    /// coverage all read this box. Fill and Stretch fill the canvas (Fill
    /// crops); Fit and None are centred boxes of the source aspect or size.
    pub fn place(self, canvas: [f64; 2], source: [f64; 2]) -> Option<FitBox> {
        let [width, height] = self.extent(canvas, source)?;
        Some(match self {
            Self::Fill | Self::Stretch | Self::Manual => FitBox {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                cover: self == Self::Fill,
            },
            Self::Fit | Self::None => FitBox {
                x: (1.0 - width) / 2.0,
                y: (1.0 - height) / 2.0,
                width,
                height,
                cover: false,
            },
        })
    }
}

/// Pixel size of a video or image item's cropped source region, from its
/// asset's first video stream; `None` for other items or a missing stream.
pub fn cropped_source_size(assets: &[media::SourceAsset], item: &crate::Item) -> Option<[f64; 2]> {
    let crate::ItemParams::Video(params) = &item.content else {
        return None;
    };
    let shape = assets
        .iter()
        .find(|asset| Some(asset.id()) == item.source_asset)?
        .probe()
        .streams
        .iter()
        .find_map(|stream| stream.video.as_ref())?;
    let crop = params.crop.unwrap_or_default();
    Some([
        f64::from(shape.width) * crop.width,
        f64::from(shape.height) * crop.height,
    ])
}

/// The canvas box of a video or image item under its fit mode; `None` for other
/// items. Without a known source size the item fills the canvas, as Fill does.
pub fn fit_box(
    canvas: &crate::Canvas,
    assets: &[media::SourceAsset],
    item: &crate::Item,
) -> Option<FitBox> {
    let crate::ItemParams::Video(params) = &item.content else {
        return None;
    };
    let size = [f64::from(canvas.width), f64::from(canvas.height)];
    cropped_source_size(assets, item)
        .and_then(|source| params.fit.place(size, source))
        .or_else(|| FitMode::Fill.place(size, size))
}
