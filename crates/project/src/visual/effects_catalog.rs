//! The admitted effect kinds as data: id, name, shelf and parameter schema.
use super::effects::{
    EffectDefinition, EffectParamDefinition, EffectParamKind, EffectShelf, EffectValue,
};
use crate::Color;

/// Slider range of every colour-grade control.
pub const GRADE_MIN: f64 = -50.0;
pub const GRADE_MAX: f64 = 50.0;
/// Chroma-key controls: two 0..=100 sliders and one key colour.
/// Measured on the reference: intensity default 40, shadow default 0.
pub const CHROMA_KEY_MIN: f64 = 0.0;
pub const CHROMA_KEY_MAX: f64 = 100.0;
pub const CHROMA_KEY_INTENSITY_DEFAULT: f64 = 40.0;
/// Green that a key colour picker starts from.
pub const CHROMA_KEY_COLOR_DEFAULT: Color = Color([0x00, 0xFF, 0x00, 255]);

const fn slider(
    id: &'static str,
    name: &'static str,
    (min, max): (f64, f64),
    default: f64,
    animatable: bool,
) -> EffectParamDefinition {
    EffectParamDefinition {
        id,
        name,
        kind: EffectParamKind::Float {
            min,
            max,
            soft_min: min,
            soft_max: max,
        },
        default: EffectValue::Scalar(default),
        animatable,
    }
}
/// Colour grade controls share one range and neutral default (0 = identity).
const fn grade_param(id: &'static str, name: &'static str) -> EffectParamDefinition {
    slider(id, name, (GRADE_MIN, GRADE_MAX), 0.0, false)
}

static DEFINITIONS: [EffectDefinition; 3] = [
    EffectDefinition {
        id: "blur",
        name: "Blur",
        shelf: EffectShelf::BlurDetail,
        params: &[slider("intensity", "Intensity", (0.0, 100.0), 15.0, true)],
    },
    EffectDefinition {
        id: "color_grade",
        name: "Color grade",
        shelf: EffectShelf::Inspector,
        params: &[
            grade_param("brightness", "Brightness"),
            grade_param("contrast", "Contrast"),
            grade_param("saturation", "Saturation"),
            grade_param("hue", "Hue"),
        ],
    },
    EffectDefinition {
        id: "chroma_key",
        name: "Chroma key",
        shelf: EffectShelf::Inspector,
        params: &[
            EffectParamDefinition {
                id: "key_color",
                name: "Key color",
                kind: EffectParamKind::Color,
                default: EffectValue::Color(CHROMA_KEY_COLOR_DEFAULT),
                animatable: false,
            },
            slider(
                "intensity",
                "Intensity",
                (CHROMA_KEY_MIN, CHROMA_KEY_MAX),
                CHROMA_KEY_INTENSITY_DEFAULT,
                true,
            ),
            slider(
                "shadow",
                "Shadow",
                (CHROMA_KEY_MIN, CHROMA_KEY_MAX),
                0.0,
                true,
            ),
        ],
    },
];

pub fn effect_definitions() -> &'static [EffectDefinition] {
    &DEFINITIONS
}
