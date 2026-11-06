//! Persisted effect instances; evaluation and devices belong to render/apps.
use crate::{AnimationPath, Error, Result};
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
}
impl Default for EffectParams {
    fn default() -> Self {
        Self::Blur {
            intensity: BLUR_INTENSITY.default,
        }
    }
}
impl EffectParams {
    /// Change one registered scalar, refusing unknown keys and invalid values atomically.
    pub fn set_scalar(&mut self, key: &str, value: f64) -> Result<()> {
        let candidate = match (&*self, key) {
            (Self::Blur { .. }, "intensity") => Self::Blur { intensity: value },
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
            Self::ColorGrade { .. } => Err(Error::Invalid(
                "colour grade controls must be finite and in -50..=50",
            )),
            Self::Blur { .. } => Err(Error::Invalid(
                "blur intensity must be finite and in 0..=100",
            )),
        }
    }
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
