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
    range: (f64, f64),
    default: f64,
    animatable: bool,
) -> EffectParamDefinition {
    ranged(id, name, range, range, default, animatable)
}
/// A slider whose hard bound clamps typed values and whose soft range is what
/// the knob spans.
const fn ranged(
    id: &'static str,
    name: &'static str,
    (min, max): (f64, f64),
    (soft_min, soft_max): (f64, f64),
    default: f64,
    animatable: bool,
) -> EffectParamDefinition {
    EffectParamDefinition {
        id,
        name,
        kind: EffectParamKind::Float {
            min,
            max,
            soft_min,
            soft_max,
        },
        default: EffectValue::Scalar(default),
        animatable,
    }
}
const fn choice(
    id: &'static str,
    name: &'static str,
    options: &'static [&'static str],
    default: u32,
) -> EffectParamDefinition {
    EffectParamDefinition {
        id,
        name,
        kind: EffectParamKind::Choice { options },
        default: EffectValue::Choice(default),
        animatable: false,
    }
}
const fn flag(id: &'static str, name: &'static str, default: bool) -> EffectParamDefinition {
    EffectParamDefinition {
        id,
        name,
        kind: EffectParamKind::Bool,
        default: EffectValue::Bool(default),
        animatable: false,
    }
}
const fn colour(id: &'static str, name: &'static str, default: Color) -> EffectParamDefinition {
    EffectParamDefinition {
        id,
        name,
        kind: EffectParamKind::Color,
        default: EffectValue::Color(default),
        animatable: false,
    }
}
/// Colour grade controls share one range and neutral default (0 = identity).
const fn grade_param(id: &'static str, name: &'static str) -> EffectParamDefinition {
    slider(id, name, (GRADE_MIN, GRADE_MAX), 0.0, false)
}
const BLACK: Color = Color([0, 0, 0, 255]);
const WHITE: Color = Color([255, 255, 255, 255]);
const RED: Color = Color([255, 0, 0, 255]);
const BLUE: Color = Color([0, 0, 255, 255]);

const INVERT_CHANNELS: &[&str] = &["RGB", "Red", "Green", "Blue", "Alpha"];

static DEFINITIONS: &[EffectDefinition] = &[
    EffectDefinition {
        id: "blur",
        name: "Blur",
        shelf: EffectShelf::BlurDetail,
        params: &[slider("intensity", "Intensity", (0.0, 100.0), 15.0, true)],
    },
    EffectDefinition {
        id: "black_white",
        name: "Black & white",
        shelf: EffectShelf::Colour,
        params: &[],
    },
    EffectDefinition {
        id: "channel_mix",
        name: "Channel mix",
        shelf: EffectShelf::Colour,
        params: &[
            slider("rr", "Red from red", (-200.0, 200.0), 100.0, true),
            slider("rg", "Red from green", (-200.0, 200.0), 0.0, true),
            slider("rb", "Red from blue", (-200.0, 200.0), 0.0, true),
            slider("rc", "Red constant", (-200.0, 200.0), 0.0, true),
            slider("gr", "Green from red", (-200.0, 200.0), 0.0, true),
            slider("gg", "Green from green", (-200.0, 200.0), 100.0, true),
            slider("gb", "Green from blue", (-200.0, 200.0), 0.0, true),
            slider("gc", "Green constant", (-200.0, 200.0), 0.0, true),
            slider("br", "Blue from red", (-200.0, 200.0), 0.0, true),
            slider("bg", "Blue from green", (-200.0, 200.0), 0.0, true),
            slider("bb", "Blue from blue", (-200.0, 200.0), 100.0, true),
            slider("bc", "Blue constant", (-200.0, 200.0), 0.0, true),
        ],
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
        id: "color_pass",
        name: "Keep colour",
        shelf: EffectShelf::Colour,
        params: &[
            colour("color", "Colour", RED),
            slider("similarity", "Similarity", (0.0, 100.0), 10.0, true),
            flag("reverse", "Reverse", false),
        ],
    },
    EffectDefinition {
        id: "color_replace",
        name: "Swap colour",
        shelf: EffectShelf::Colour,
        params: &[
            slider("similarity", "Similarity", (0.0, 100.0), 10.0, true),
            flag("solid", "Solid colour", false),
            colour("target", "Target colour", RED),
            colour("replace", "Replace with", BLUE),
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
    // Test-only kind: no shelf lists it, and no production build contains it.
    // Render's tests use it to prove the content-time remap and the preview
    // quality input through the same evaluation path as a real effect.
    #[cfg(feature = "test-kinds")]
    EffectDefinition {
        id: "hold_time",
        name: "Hold time (test)",
        shelf: EffectShelf::Inspector,
        params: &[slider("rate", "Frame rate", (1.0, 60.0), 2.0, true)],
    },
    EffectDefinition {
        id: "extract",
        name: "Threshold",
        shelf: EffectShelf::Stylize,
        params: &[
            slider("black", "Black point", (0.0, 255.0), 0.0, true),
            slider("white", "White point", (0.0, 255.0), 255.0, true),
            slider("softness", "Softness", (0.0, 100.0), 0.0, true),
            flag("invert", "Invert", false),
        ],
    },
    EffectDefinition {
        id: "gamma_correction",
        name: "Gamma",
        shelf: EffectShelf::Colour,
        params: &[slider("gamma", "Gamma", (1.0, 28.0), 10.0, true)],
    },
    EffectDefinition {
        id: "invert",
        name: "Invert",
        shelf: EffectShelf::Colour,
        params: &[
            choice("channel", "Channel", INVERT_CHANNELS, 0),
            slider("blend", "Blend with original", (0.0, 100.0), 0.0, true),
        ],
    },
    EffectDefinition {
        id: "levels",
        name: "Levels",
        shelf: EffectShelf::Colour,
        params: &[
            slider("in_black", "Input black", (0.0, 255.0), 0.0, true),
            slider("in_white", "Input white", (0.0, 255.0), 255.0, true),
            slider("out_black", "Output black", (0.0, 255.0), 0.0, true),
            slider("out_white", "Output white", (0.0, 255.0), 255.0, true),
            ranged("gamma", "Gamma", (10.0, 1000.0), (10.0, 300.0), 100.0, true),
        ],
    },
    EffectDefinition {
        id: "posterize",
        name: "Posterise",
        shelf: EffectShelf::Stylize,
        params: &[ranged(
            "levels",
            "Levels",
            (2.0, 255.0),
            (2.0, 32.0),
            7.0,
            true,
        )],
    },
    EffectDefinition {
        id: "strobe",
        name: "Strobe",
        shelf: EffectShelf::Stylize,
        params: &[
            colour("color", "Strobe colour", WHITE),
            slider("blend", "Blend with original", (0.0, 100.0), 0.0, true),
            ranged("duration", "Duration", (0.0, 30.0), (0.0, 1.0), 0.05, true),
            ranged("period", "Period", (0.0, 30.0), (0.0, 2.0), 0.5, true),
        ],
    },
    EffectDefinition {
        id: "tint",
        name: "Two-tone",
        shelf: EffectShelf::Colour,
        params: &[
            colour("black", "Map black to", BLACK),
            colour("white", "Map white to", WHITE),
            slider("amount", "Amount", (0.0, 100.0), 100.0, true),
        ],
    },
];

pub fn effect_definitions() -> &'static [EffectDefinition] {
    DEFINITIONS
}
