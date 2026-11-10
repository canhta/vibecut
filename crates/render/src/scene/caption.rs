//! Caption typography and safe-area placement.

use super::{Fit, Placement, Rect};
use crate::{Error, Result};
use captions::{CueStyle, VerticalAlignment};
use project::{TextAlign, TextDecoration};
use serde::{Deserialize, Serialize};

/// ASS records no outline width in the cue style table, so an imported outline
/// keeps the default 1080-line width instead of inventing one.
const IMPORTED_OUTLINE_WIDTH: f32 = 2.0;
/// Per-side ceiling shared with [`CaptionStyle::safe_area_percent`], so a hostile
/// margin can never produce an empty destination.
const MAX_CUE_MARGIN: f64 = 0.4;

/// Vertical anchor inside the caption destination after its safe-area inset.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CaptionPosition {
    Top,
    Middle,
    #[default]
    Bottom,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionOutline {
    pub color: [u8; 4],
    /// Pixels at 1080 output lines, scaled with output height.
    pub width: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionBackground {
    pub color: [u8; 4],
    /// Pixels at 1080 output lines.
    pub padding: f32,
    pub radius: f32,
}
/// Transient render settings, never a second persisted cue/style schema.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionStyle {
    pub font: String,
    pub font_style: String,
    /// Pixels at 1080 output lines, independent of aspect ratio.
    pub size: f32,
    pub color: [u8; 4],
    pub outline: Option<CaptionOutline>,
    pub background: Option<CaptionBackground>,
    pub position: CaptionPosition,
    /// Percent inset on each canvas edge (0..=40), also applied in portrait output.
    pub safe_area_percent: f32,
    /// Total lines across both languages; wrapped text shrinks to fit without dropping text.
    pub max_lines: usize,
}
impl Default for CaptionStyle {
    fn default() -> Self {
        Self {
            font: "Inter".into(),
            font_style: "SemiBold".into(),
            size: 48.0,
            color: [255; 4],
            outline: Some(CaptionOutline {
                color: [0, 0, 0, 255],
                width: 2.0,
            }),
            background: None,
            position: CaptionPosition::Bottom,
            safe_area_percent: 5.0,
            max_lines: 2,
        }
    }
}
impl CaptionStyle {
    /// Project settings with one imported cue's static style applied. A cue
    /// without a style keeps the settings untouched.
    pub(crate) fn for_cue(&self, cue: Option<&CueStyle>) -> Self {
        let Some(cue) = cue else {
            return self.clone();
        };
        Self {
            font: cue.font_family.as_str().to_owned(),
            font_style: text::style_name(if cue.bold { 700 } else { 400 }, cue.italic),
            // The stored ratio is the ASS font size over PlayResY; caption sizes
            // are pixels at 1080 output lines.
            size: cue.font_size_ratio * 1080.0,
            color: cue.primary_color,
            outline: (cue.border_style != 3).then(|| CaptionOutline {
                color: cue.outline_color,
                width: IMPORTED_OUTLINE_WIDTH,
            }),
            background: (cue.border_style == 3).then(|| CaptionBackground {
                color: cue.background_color,
                padding: 0.0,
                radius: 0.0,
            }),
            position: match cue.alignment.vertical {
                VerticalAlignment::Top => CaptionPosition::Top,
                VerticalAlignment::Middle => CaptionPosition::Middle,
                VerticalAlignment::Bottom => CaptionPosition::Bottom,
            },
            safe_area_percent: self.safe_area_percent,
            max_lines: self.max_lines,
        }
    }

    /// Destination band for one cue. A styled cue keeps its per-side ASS margins;
    /// an unstyled cue keeps the uniform safe-area inset. Both share the same
    /// bounded line-height band.
    pub(crate) fn cue_placement(&self, cue: Option<&CueStyle>) -> Placement {
        let (left, right, vertical) = cue.map_or_else(
            || {
                let inset = f64::from(self.safe_area_percent) / 100.0;
                (inset, inset, inset)
            },
            |cue| {
                (
                    bounded_margin(cue.margin_left_ratio),
                    bounded_margin(cue.margin_right_ratio),
                    bounded_margin(cue.margin_vertical_ratio),
                )
            },
        );
        let decoration = self
            .outline
            .as_ref()
            .map_or(0.0, |o| o.width)
            .max(self.background.as_ref().map_or(0.0, |b| b.padding));
        let height = ((f64::from(self.size) * self.max_lines as f64 * 2.0
            + f64::from(decoration) * 2.0
            + 4.0)
            / 1080.0)
            .min(1.0 - 2.0 * vertical);
        let y = match self.position {
            CaptionPosition::Top => vertical,
            CaptionPosition::Middle => (1.0 - height) / 2.0,
            CaptionPosition::Bottom => 1.0 - vertical - height,
        };
        Placement {
            destination: Rect {
                x: left,
                y,
                width: 1.0 - left - right,
                height,
            },
            fit: Fit::Stretch,
            ..Default::default()
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        let bounded = |v: f32, max: f32| v.is_finite() && (0.0..=max).contains(&v);
        if self.font.len() > 256
            || self.font_style.len() > 256
            || !bounded(self.size, 256.0)
            || self.size < 4.0
            || !bounded(self.safe_area_percent, 40.0)
            || !(1..=16).contains(&self.max_lines)
            || self
                .outline
                .as_ref()
                .is_some_and(|o| !bounded(o.width, 64.0))
            || self
                .background
                .as_ref()
                .is_some_and(|b| !bounded(b.padding, 128.0) || !bounded(b.radius, 128.0))
        {
            return Err(Error::Invalid("caption style limits"));
        }
        Ok(())
    }
}

fn bounded_margin(ratio: f32) -> f64 {
    f64::from(ratio).clamp(0.0, MAX_CUE_MARGIN)
}

/// Horizontal placement of a cue's text inside its band.
pub(super) fn align(cue: Option<&CueStyle>) -> TextAlign {
    match cue.map(|cue| cue.alignment.horizontal) {
        Some(captions::HorizontalAlignment::Left) => TextAlign::Left,
        Some(captions::HorizontalAlignment::Right) => TextAlign::Right,
        _ => TextAlign::Center,
    }
}

/// The one line a cue style draws; underline wins when both are set.
pub(super) fn decoration(cue: Option<&CueStyle>) -> TextDecoration {
    match cue {
        Some(cue) if cue.underline => TextDecoration::Underline,
        Some(cue) if cue.strikeout => TextDecoration::LineThrough,
        _ => TextDecoration::None,
    }
}
