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
pub fn effect_definitions() -> &'static [EffectDefinition] {
    &[EffectDefinition {
        kind: "blur",
        name: "Blur",
        params: &[BLUR_INTENSITY],
    }]
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
    Blur { intensity: f64 },
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
            _ => Err(Error::Invalid(
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
