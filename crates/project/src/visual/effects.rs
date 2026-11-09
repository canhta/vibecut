//! Persisted effect instances; evaluation and devices belong to render/apps.
use crate::{AnimationPath, Color, Error, Result};
use serde::{Deserialize, Serialize};

/// UI/catalog metadata lives beside the typed persisted parameters.
#[derive(Clone, Copy, Debug)]
pub struct EffectDefinition {
    pub kind: &'static str,
    pub name: &'static str,
    pub params: &'static [EffectParamDefinition],
}
#[derive(Clone, Copy, Debug)]
pub struct EffectParamDefinition {
    pub key: &'static str,
    pub name: &'static str,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub animatable: bool,
}
const BLUR_INTENSITY: EffectParamDefinition = EffectParamDefinition {
    key: "intensity",
    name: "Intensity",
    min: 0.0,
    max: 100.0,
    default: 15.0,
    animatable: true,
};
/// Colour grade controls share one range and neutral default (0 = identity).
const fn grade_param(key: &'static str, name: &'static str) -> EffectParamDefinition {
    EffectParamDefinition {
        key,
        name,
        min: GRADE_MIN,
        max: GRADE_MAX,
        default: 0.0,
        animatable: false,
    }
}
/// Slider range of every colour-grade control.
pub const GRADE_MIN: f64 = -50.0;
pub const GRADE_MAX: f64 = 50.0;
const GRADE_PARAMS: [EffectParamDefinition; 4] = [
    grade_param("brightness", "Brightness"),
    grade_param("contrast", "Contrast"),
    grade_param("saturation", "Saturation"),
    grade_param("hue", "Hue"),
];
/// Chroma-key controls: two 0..=100 sliders and one packed key colour.
/// Measured on the reference: intensity default 40, shadow default 0.
pub const CHROMA_KEY_MIN: f64 = 0.0;
pub const CHROMA_KEY_MAX: f64 = 100.0;
pub const CHROMA_KEY_INTENSITY_DEFAULT: f64 = 40.0;
/// Silver-green that a key colour picker starts from.
pub const CHROMA_KEY_COLOR_DEFAULT: f64 = 0x00FF00 as f64;
/// A packed key colour is exactly one 24-bit sRGB value.
pub const CHROMA_KEY_COLOR_MAX: f64 = 0xFFFFFF as f64;
const CHROMA_KEY_PARAMS: [EffectParamDefinition; 3] = [
    EffectParamDefinition {
        key: "key_color",
        name: "Key color",
        min: 0.0,
        max: CHROMA_KEY_COLOR_MAX,
        default: CHROMA_KEY_COLOR_DEFAULT,
        animatable: false,
    },
    EffectParamDefinition {
        key: "intensity",
        name: "Intensity",
        min: CHROMA_KEY_MIN,
        max: CHROMA_KEY_MAX,
        default: CHROMA_KEY_INTENSITY_DEFAULT,
        animatable: false,
    },
    EffectParamDefinition {
        key: "shadow",
        name: "Shadow",
        min: CHROMA_KEY_MIN,
        max: CHROMA_KEY_MAX,
        default: 0.0,
        animatable: false,
    },
];
pub fn effect_definitions() -> &'static [EffectDefinition] {
    &[
        EffectDefinition {
            kind: "blur",
            name: "Blur",
            params: &[BLUR_INTENSITY],
        },
        EffectDefinition {
            kind: "color_grade",
            name: "Color grade",
            params: &GRADE_PARAMS,
        },
        EffectDefinition {
            kind: "chroma_key",
            name: "Chroma key",
            params: &CHROMA_KEY_PARAMS,
        },
    ]
}

/// Stable instance identity is local to an item and also names its animation paths.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectInstance {
    pub id: String,
    pub enabled: bool,
    pub params: EffectParams,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EffectParams {
    Blur {
        intensity: f64,
    },
    /// Primary grade; every control is -50..=50 and 0 is identity.
    ColorGrade {
        brightness: f64,
        contrast: f64,
        saturation: f64,
        hue: f64,
    },
    /// Removes the picked key colour; intensity 0 keys nothing.
    ChromaKey {
        key_color: Color,
        intensity: f64,
        shadow: f64,
    },
}
impl Default for EffectParams {
    fn default() -> Self {
        Self::Blur {
            intensity: BLUR_INTENSITY.default,
        }
    }
}
impl EffectParams {
    /// Catalog kind, so callers never match over every variant.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Blur { .. } => "blur",
            Self::ColorGrade { .. } => "color_grade",
            Self::ChromaKey { .. } => "chroma_key",
        }
    }

    /// Change one registered scalar, refusing unknown keys and invalid values atomically.
    /// A chroma key colour arrives as one packed 0xRRGGBB scalar so a pick is a
    /// single effect command.
    pub fn set_scalar(&mut self, key: &str, value: f64) -> Result<()> {
        let candidate = match (&*self, key) {
            (Self::Blur { .. }, "intensity") => Self::Blur { intensity: value },
            (Self::ChromaKey { .. }, "key_color") => {
                let packed = chroma_key_color(value)?;
                let mut next = self.clone();
                if let Self::ChromaKey { key_color, .. } = &mut next {
                    *key_color = packed;
                }
                next
            }
            (Self::ChromaKey { .. }, "intensity" | "shadow") => {
                let mut next = self.clone();
                if let Self::ChromaKey {
                    intensity, shadow, ..
                } = &mut next
                {
                    match key {
                        "intensity" => *intensity = value,
                        _ => *shadow = value,
                    }
                }
                next
            }
            (Self::ColorGrade { .. }, "brightness" | "contrast" | "saturation" | "hue") => {
                let mut next = self.clone();
                if let Self::ColorGrade {
                    brightness,
                    contrast,
                    saturation,
                    hue,
                } = &mut next
                {
                    match key {
                        "brightness" => *brightness = value,
                        "contrast" => *contrast = value,
                        "saturation" => *saturation = value,
                        _ => *hue = value,
                    }
                }
                next
            }
            _ => return Err(Error::Unsupported("effect parameter")),
        };
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Blur { intensity }
                if intensity.is_finite()
                    && (BLUR_INTENSITY.min..=BLUR_INTENSITY.max).contains(intensity) =>
            {
                Ok(())
            }
            Self::ColorGrade {
                brightness,
                contrast,
                saturation,
                hue,
            } if [brightness, contrast, saturation, hue]
                .iter()
                .all(|v| v.is_finite() && (GRADE_MIN..=GRADE_MAX).contains(*v)) =>
            {
                Ok(())
            }
            Self::ChromaKey {
                intensity, shadow, ..
            } if [intensity, shadow]
                .iter()
                .all(|v| v.is_finite() && (CHROMA_KEY_MIN..=CHROMA_KEY_MAX).contains(*v)) =>
            {
                Ok(())
            }
            Self::ColorGrade { .. } => Err(Error::Invalid(
                "colour grade controls must be finite and in -50..=50",
            )),
            Self::ChromaKey { .. } => Err(Error::Invalid(
                "chroma key intensity and shadow must be finite and in 0..=100",
            )),
            Self::Blur { .. } => Err(Error::Invalid(
                "blur intensity must be finite and in 0..=100",
            )),
        }
    }
}

/// One exact 24-bit sRGB colour packed into a scalar, opaque.
pub fn chroma_key_color(value: f64) -> Result<Color> {
    if !value.is_finite() || !(0.0..=CHROMA_KEY_COLOR_MAX).contains(&value) || value.fract() != 0.0
    {
        return Err(Error::Invalid("chroma key colour"));
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
    pub fn intensity_path(&self) -> Result<AnimationPath> {
        AnimationPath::new(format!("effects.{}.params.intensity", self.id))
    }
    pub(crate) fn validate(&self) -> Result<()> {
        self.intensity_path()?;
        self.params.validate()
    }
}
