//! Caption typography and safe-area placement.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};

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
