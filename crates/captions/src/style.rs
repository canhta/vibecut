//! Persisted caption presentation style with sparse per-item overrides.

use serde::{Deserialize, Serialize};

/// Distinguishes an absent optional override from an explicit null clearing it.
mod double_option {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S, T>(value: &Option<Option<T>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize,
    {
        match value {
            Some(inner) => inner.serialize(serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        Option::<T>::deserialize(deserializer).map(Some)
    }
}

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
    /// Stroke thickness on the 0..=100 scale text strokes use; [`outline_width`] gives the
    /// pixels at the caption's size.
    pub thickness: f32,
}

/// Largest stroke thickness, shared by text strokes and caption outlines.
pub const MAX_THICKNESS: f32 = 100.0;
/// Stroke width at thickness 100 as a share of the font size. The reference gives no pixel
/// measure for its 0..=100 Thickness, so this proportional rule is an open value.
pub const THICKNESS_FONT_RATIO: f32 = 0.2;

/// Outer stroke width in the units of `font_size` for a 0..=100 thickness.
pub fn outline_width(thickness: f32, font_size: f32) -> f32 {
    thickness / MAX_THICKNESS * THICKNESS_FONT_RATIO * font_size
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionBackground {
    pub color: [u8; 4],
    /// Pixels at 1080 output lines.
    pub padding: f32,
    pub radius: f32,
}

/// Complete caption presentation: the saved project value and the transient
/// render default share this one vocabulary.
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
    /// RGB of the word-highlight overlay: the active spoken word and the keyword-emphasis
    /// words. Alpha stays the caption colour's alpha.
    pub word_highlight_color: [u8; 3],
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
                thickness: 20.0,
            }),
            background: None,
            position: CaptionPosition::Bottom,
            safe_area_percent: 5.0,
            max_lines: 2,
            word_highlight_color: [255, 215, 0],
        }
    }
}

impl CaptionStyle {
    /// Render and persistence guard for hostile finite values.
    pub fn is_valid(&self) -> bool {
        let bounded = |v: f32, max: f32| v.is_finite() && (0.0..=max).contains(&v);
        !self.font.trim().is_empty()
            && self.font.len() <= 256
            && !self.font_style.trim().is_empty()
            && self.font_style.len() <= 256
            && bounded(self.size, 256.0)
            && self.size >= 4.0
            && bounded(self.safe_area_percent, 40.0)
            && (1..=16).contains(&self.max_lines)
            && self
                .outline
                .as_ref()
                .is_none_or(|o| bounded(o.thickness, MAX_THICKNESS))
            && self
                .background
                .as_ref()
                .is_none_or(|b| bounded(b.padding, 128.0) && bounded(b.radius, 128.0))
    }
}

/// Sparse per-item patch over a selected saved preset. Unset fields keep the
/// preset value; an explicit null clears an optional outline or box.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 4]>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "double_option::serialize",
        deserialize_with = "double_option::deserialize"
    )]
    pub outline: Option<Option<CaptionOutline>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "double_option::serialize",
        deserialize_with = "double_option::deserialize"
    )]
    pub background: Option<Option<CaptionBackground>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<CaptionPosition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safe_area_percent: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_lines: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_highlight_color: Option<[u8; 3]>,
}

impl CaptionOverrides {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Fold every set field over `style`; unset fields are untouched.
    pub fn apply(&self, style: &mut CaptionStyle) {
        if let Some(value) = &self.font {
            style.font.clone_from(value);
        }
        if let Some(value) = &self.font_style {
            style.font_style.clone_from(value);
        }
        if let Some(value) = self.size {
            style.size = value;
        }
        if let Some(value) = self.color {
            style.color = value;
        }
        if let Some(value) = &self.outline {
            style.outline.clone_from(value);
        }
        if let Some(value) = &self.background {
            style.background.clone_from(value);
        }
        if let Some(value) = self.position {
            style.position = value;
        }
        if let Some(value) = self.safe_area_percent {
            style.safe_area_percent = value;
        }
        if let Some(value) = self.max_lines {
            style.max_lines = value;
        }
        if let Some(value) = self.word_highlight_color {
            style.word_highlight_color = value;
        }
    }

    /// Every set field stays within the complete style bounds.
    pub fn is_valid(&self) -> bool {
        let mut style = CaptionStyle::default();
        self.apply(&mut style);
        style.is_valid()
    }
}
