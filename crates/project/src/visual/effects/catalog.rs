//! The admitted effect kinds as data: id, name, shelf and parameter schema.
use super::{EffectDefinition, EffectParamDefinition, EffectParamKind, EffectShelf, EffectValue};
use crate::{Color, ToneCurve};

/// Slider range of every basic colour-grade control.
pub const GRADE_MIN: f64 = -50.0;
pub const GRADE_MAX: f64 = 50.0;
/// The Basic tab's controls in reference order: Color, Lightness, Effects.
pub const GRADE_BASIC: [&str; 12] = [
    "saturation",
    "temperature",
    "hue",
    "brightness",
    "contrast",
    "shine",
    "highlight",
    "shadow",
    "sharpness",
    "vignette",
    "fade",
    "grain",
];
/// The HSL tab's hue bands in reference order.
pub const HSL_BANDS: [&str; 8] = [
    "red", "orange", "yellow", "green", "cyan", "blue", "purple", "magenta",
];
/// One key per HSL band control, in the tab's row order.
pub const HSL_CONTROLS: [&str; 3] = ["hue", "saturation", "lightness"];
/// Lift is a signed straight-colour offset in percent.
pub const LIFT_MIN: f64 = -100.0;
pub const LIFT_MAX: f64 = 100.0;
/// Gamma is a positive straight-colour power in percent, 100 neutral.
pub const GAMMA_MIN: f64 = 25.0;
pub const GAMMA_MAX: f64 = 400.0;
/// Gain is a straight-colour multiplier in percent, 100 neutral.
pub const GAIN_MIN: f64 = 0.0;
pub const GAIN_MAX: f64 = 400.0;
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
        animatable: true,
    }
}
const fn point(id: &'static str, name: &'static str, [x, y]: [f64; 2]) -> EffectParamDefinition {
    EffectParamDefinition {
        id,
        name,
        kind: EffectParamKind::Point,
        default: EffectValue::Point([x, y]),
        animatable: false,
    }
}
const fn angle(id: &'static str, name: &'static str, default: f64) -> EffectParamDefinition {
    EffectParamDefinition {
        id,
        name,
        kind: EffectParamKind::Angle,
        default: EffectValue::Scalar(default),
        animatable: true,
    }
}
/// Colour grade controls share one range and neutral default (0 = identity).
const fn grade_param(id: &'static str, name: &'static str) -> EffectParamDefinition {
    slider(id, name, (GRADE_MIN, GRADE_MAX), 0.0, true)
}
/// One static tone-curve parameter; the neutral value is the two endpoints.
const fn curve_param(id: &'static str, name: &'static str) -> EffectParamDefinition {
    EffectParamDefinition {
        id,
        name,
        kind: EffectParamKind::Curve,
        default: EffectValue::Curve(ToneCurve::IDENTITY),
        animatable: false,
    }
}
const BLACK: Color = Color([0, 0, 0, 255]);
const WHITE: Color = Color([255, 255, 255, 255]);
const RED: Color = Color([255, 0, 0, 255]);
const BLUE: Color = Color([0, 0, 255, 255]);

const INVERT_CHANNELS: &[&str] = &["RGB", "Red", "Green", "Blue", "Alpha"];
const MAGNIFY_SHAPES: &[&str] = &["Circle", "Square"];
const TURBULENT_KINDS: &[&str] = &[
    "Turbulent",
    "Bulge",
    "Twist",
    "Turbulent Smoother",
    "Bulge Smoother",
    "Twist Smoother",
    "Vertical Displacement",
    "Horizontal Displacement",
    "Cross Displacement",
];
const GLINT_RAYS: &[&str] = &["2", "4", "6", "8"];
const ADD_SCREEN: &[&str] = &["Add", "Screen"];
const SCREEN_ADD_OVERLAY: &[&str] = &["Screen", "Add", "Overlay"];
const SPLIT_MODES: &[&str] = &["Linear", "Radial"];
const TORN_EDGES: &[&str] = &[
    "Roughen",
    "Roughen Colour",
    "Cut",
    "Spiky",
    "Rusty",
    "Rusty Colour",
    "Photocopy",
    "Photocopy Colour",
];
const GRADIENT_SHAPES: &[&str] = &["Linear", "Radial", "Reflected", "Diamond"];

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
            grade_param("temperature", "Temperature"),
            grade_param("highlight", "Highlight"),
            grade_param("shadow", "Shadow"),
            grade_param("shine", "Shine"),
            grade_param("sharpness", "Sharpness"),
            grade_param("vignette", "Vignette"),
            grade_param("fade", "Fade"),
            grade_param("grain", "Grain"),
            grade_param("hsl_red_hue", "Red hue"),
            grade_param("hsl_red_saturation", "Red saturation"),
            grade_param("hsl_red_lightness", "Red lightness"),
            grade_param("hsl_orange_hue", "Orange hue"),
            grade_param("hsl_orange_saturation", "Orange saturation"),
            grade_param("hsl_orange_lightness", "Orange lightness"),
            grade_param("hsl_yellow_hue", "Yellow hue"),
            grade_param("hsl_yellow_saturation", "Yellow saturation"),
            grade_param("hsl_yellow_lightness", "Yellow lightness"),
            grade_param("hsl_green_hue", "Green hue"),
            grade_param("hsl_green_saturation", "Green saturation"),
            grade_param("hsl_green_lightness", "Green lightness"),
            grade_param("hsl_cyan_hue", "Cyan hue"),
            grade_param("hsl_cyan_saturation", "Cyan saturation"),
            grade_param("hsl_cyan_lightness", "Cyan lightness"),
            grade_param("hsl_blue_hue", "Blue hue"),
            grade_param("hsl_blue_saturation", "Blue saturation"),
            grade_param("hsl_blue_lightness", "Blue lightness"),
            grade_param("hsl_purple_hue", "Purple hue"),
            grade_param("hsl_purple_saturation", "Purple saturation"),
            grade_param("hsl_purple_lightness", "Purple lightness"),
            grade_param("hsl_magenta_hue", "Magenta hue"),
            grade_param("hsl_magenta_saturation", "Magenta saturation"),
            grade_param("hsl_magenta_lightness", "Magenta lightness"),
            slider("lift_r", "Lift red", (LIFT_MIN, LIFT_MAX), 0.0, true),
            slider("lift_g", "Lift green", (LIFT_MIN, LIFT_MAX), 0.0, true),
            slider("lift_b", "Lift blue", (LIFT_MIN, LIFT_MAX), 0.0, true),
            slider("gamma_r", "Gamma red", (GAMMA_MIN, GAMMA_MAX), 100.0, true),
            slider(
                "gamma_g",
                "Gamma green",
                (GAMMA_MIN, GAMMA_MAX),
                100.0,
                true,
            ),
            slider("gamma_b", "Gamma blue", (GAMMA_MIN, GAMMA_MAX), 100.0, true),
            slider("gain_r", "Gain red", (GAIN_MIN, GAIN_MAX), 100.0, true),
            slider("gain_g", "Gain green", (GAIN_MIN, GAIN_MAX), 100.0, true),
            slider("gain_b", "Gain blue", (GAIN_MIN, GAIN_MAX), 100.0, true),
            curve_param("curve_red", "Red curve"),
            curve_param("curve_green", "Green curve"),
            curve_param("curve_blue", "Blue curve"),
            curve_param("curve_luma", "Luma curve"),
            EffectParamDefinition {
                id: "lut",
                name: "LUT",
                kind: EffectParamKind::Lut,
                default: EffectValue::Lut(None),
                animatable: false,
            },
            slider("lut_strength", "LUT strength", (0.0, 100.0), 100.0, false),
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
        id: "directional_blur",
        name: "Streak Blur",
        shelf: EffectShelf::BlurDetail,
        params: &[
            EffectParamDefinition {
                id: "direction",
                name: "Direction",
                kind: EffectParamKind::Angle,
                default: EffectValue::Scalar(0.0),
                animatable: true,
            },
            ranged("length", "Length", (0.0, 1000.0), (0.0, 20.0), 0.0, true),
        ],
    },
    EffectDefinition {
        id: "sharpen",
        name: "Sharpen",
        shelf: EffectShelf::BlurDetail,
        params: &[
            // Source amount is a percent gain: 100 keeps one unit of detail
            // difference, 4000 is the hard ceiling.
            ranged("amount", "Amount", (0.0, 4000.0), (0.0, 100.0), 0.0, true),
        ],
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
        id: "mosaic",
        name: "Pixelate",
        shelf: EffectShelf::Stylize,
        params: &[
            // Source counts are block counts, not pixel sizes: 1 is one block per
            // axis and 4000 is the hard ceiling.
            ranged(
                "horizontal",
                "Horizontal",
                (1.0, 4000.0),
                (1.0, 200.0),
                10.0,
                true,
            ),
            ranged(
                "vertical",
                "Vertical",
                (1.0, 4000.0),
                (1.0, 200.0),
                10.0,
                true,
            ),
            flag("sharp", "Sharp", false),
            slider("softness", "Softness", (0.0, 100.0), 0.0, true),
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
    EffectDefinition {
        id: "find_edges",
        name: "Edge Lines",
        shelf: EffectShelf::Stylize,
        params: &[
            flag("invert", "Invert", false),
            slider("blend", "Blend with original", (0.0, 100.0), 0.0, true),
        ],
    },
    EffectDefinition {
        id: "color_emboss",
        name: "Colour Relief",
        shelf: EffectShelf::Stylize,
        params: &[
            EffectParamDefinition {
                id: "direction",
                name: "Direction",
                kind: EffectParamKind::Angle,
                default: EffectValue::Scalar(45.0),
                animatable: true,
            },
            slider("relief", "Relief", (0.0, 10.0), 1.0, true),
            slider("contrast", "Contrast", (0.0, 500.0), 100.0, true),
            slider("blend", "Blend with original", (0.0, 100.0), 0.0, true),
        ],
    },
    // Distort additions.
    EffectDefinition {
        id: "twirl",
        name: "Twirl",
        shelf: EffectShelf::Distort,
        params: &[
            angle("angle", "Angle", 50.0),
            slider("radius", "Twirl radius", (0.0, 100.0), 30.0, true),
            point("center", "Twirl centre", [0.5, 0.5]),
        ],
    },
    EffectDefinition {
        id: "spherize",
        name: "Bulge",
        shelf: EffectShelf::Distort,
        params: &[
            ranged("radius", "Radius", (0.0, 2500.0), (0.0, 600.0), 0.0, true),
            point("center", "Sphere centre", [0.5, 0.5]),
        ],
    },
    EffectDefinition {
        id: "magnify",
        name: "Magnifier",
        shelf: EffectShelf::Distort,
        params: &[
            choice("shape", "Shape", MAGNIFY_SHAPES, 0),
            point("center", "Centre", [0.5, 0.5]),
            ranged(
                "magnification",
                "Magnification",
                (100.0, 600.0),
                (100.0, 600.0),
                200.0,
                true,
            ),
            ranged("size", "Size", (0.0, 4000.0), (0.0, 600.0), 100.0, true),
            ranged("feather", "Feather", (0.0, 1000.0), (0.0, 200.0), 0.0, true),
        ],
    },
    EffectDefinition {
        id: "wave_warp",
        name: "Ripple",
        shelf: EffectShelf::Distort,
        params: &[
            slider("height", "Wave height", (-4000.0, 4000.0), 10.0, true),
            slider("width", "Wave width", (1.0, 4000.0), 40.0, true),
            angle("direction", "Direction", 90.0),
            slider("speed", "Wave speed", (-100.0, 100.0), 1.0, true),
        ],
    },
    EffectDefinition {
        id: "lens_distortion",
        name: "Lens Warp",
        shelf: EffectShelf::Distort,
        params: &[
            slider("curvature", "Curvature", (-100.0, 100.0), 0.0, true),
            slider(
                "v_decentering",
                "Vertical decentering",
                (-100.0, 100.0),
                0.0,
                true,
            ),
            slider(
                "h_decentering",
                "Horizontal decentering",
                (-100.0, 100.0),
                0.0,
                true,
            ),
        ],
    },
    EffectDefinition {
        id: "turbulent_displace",
        name: "Heat Haze",
        shelf: EffectShelf::Distort,
        params: &[
            choice("displacement", "Displacement", TURBULENT_KINDS, 0),
            slider("amount", "Amount", (-1000.0, 1000.0), 50.0, true),
            ranged("size", "Size", (2.0, 1000.0), (2.0, 400.0), 100.0, true),
            // The source's unset offset is the layer centre.
            point("offset", "Offset", [0.5, 0.5]),
            ranged(
                "complexity",
                "Complexity",
                (1.0, 10.0),
                (1.0, 10.0),
                1.0,
                true,
            ),
            angle("evolution", "Evolution", 0.0),
        ],
    },
    EffectDefinition {
        id: "corner_pin",
        name: "Corner Pin",
        shelf: EffectShelf::Distort,
        params: &[
            point("upper_left", "Upper left", [0.0, 0.0]),
            point("upper_right", "Upper right", [1.0, 0.0]),
            point("lower_left", "Lower left", [0.0, 1.0]),
            point("lower_right", "Lower right", [1.0, 1.0]),
        ],
    },
    EffectDefinition {
        id: "basic_3d",
        name: "Tilt Card",
        shelf: EffectShelf::Distort,
        params: &[
            angle("swivel", "Swivel", 0.0),
            angle("tilt", "Tilt", 0.0),
            slider(
                "distance",
                "Distance to image",
                (-10000.0, 10000.0),
                0.0,
                true,
            ),
        ],
    },
    // Glow & Light.
    EffectDefinition {
        id: "echo_glow",
        name: "Echo Glow",
        shelf: EffectShelf::GlowLight,
        params: &[
            slider("threshold", "Threshold", (0.0, 100.0), 60.0, true),
            ranged("echoes", "Echoes", (1.0, 12.0), (1.0, 12.0), 4.0, true),
            ranged("radius", "Radius", (0.0, 500.0), (0.0, 100.0), 12.0, true),
            ranged("spread", "Spread", (1.0, 4.0), (1.0, 4.0), 1.8, true),
            ranged(
                "intensity",
                "Intensity",
                (0.0, 1000.0),
                (0.0, 400.0),
                100.0,
                true,
            ),
            slider("decay", "Decay", (0.0, 100.0), 50.0, true),
            colour("color", "Glow colour", Color([255, 217, 153, 255])),
        ],
    },
    EffectDefinition {
        id: "edge_glow",
        name: "Edge Glow",
        shelf: EffectShelf::GlowLight,
        params: &[
            slider("threshold", "Edge threshold", (0.0, 100.0), 20.0, true),
            ranged("width", "Edge width", (0.0, 50.0), (0.0, 10.0), 2.0, true),
            ranged(
                "radius",
                "Glow radius",
                (0.0, 500.0),
                (0.0, 100.0),
                10.0,
                true,
            ),
            ranged(
                "intensity",
                "Intensity",
                (0.0, 1000.0),
                (0.0, 400.0),
                150.0,
                true,
            ),
            colour("color", "Glow colour", Color([77, 204, 255, 255])),
            flag("only", "Glow only", false),
        ],
    },
    EffectDefinition {
        id: "glint",
        name: "Sparkle",
        shelf: EffectShelf::GlowLight,
        params: &[
            slider("threshold", "Threshold", (0.0, 100.0), 75.0, true),
            choice("rays", "Rays", GLINT_RAYS, 1),
            ranged("length", "Length", (0.0, 2000.0), (0.0, 400.0), 60.0, true),
            angle("rotation", "Rotation", 45.0),
            ranged(
                "intensity",
                "Intensity",
                (0.0, 1000.0),
                (0.0, 400.0),
                100.0,
                true,
            ),
            colour("color", "Colour", WHITE),
            slider("colorize", "Colorize", (0.0, 100.0), 0.0, true),
        ],
    },
    EffectDefinition {
        id: "wonder_glow",
        name: "Bloom",
        shelf: EffectShelf::GlowLight,
        params: &[
            slider("threshold", "Threshold", (0.0, 100.0), 60.0, true),
            ranged("radius", "Radius", (0.0, 1000.0), (0.0, 200.0), 25.0, true),
            ranged(
                "intensity",
                "Intensity",
                (0.0, 1000.0),
                (0.0, 400.0),
                100.0,
                true,
            ),
            ranged(
                "saturation",
                "Saturation",
                (0.0, 400.0),
                (0.0, 200.0),
                100.0,
                true,
            ),
            flag("use_color", "Use glow colour", false),
            colour("color", "Glow colour", Color([255, 204, 128, 255])),
            choice("mode", "Blend mode", ADD_SCREEN, 0),
        ],
    },
    EffectDefinition {
        id: "volumetric_rays",
        name: "Sun Rays",
        shelf: EffectShelf::GlowLight,
        params: &[
            point("center", "Source point", [0.5, 0.25]),
            slider("threshold", "Threshold", (0.0, 100.0), 60.0, true),
            slider("length", "Ray length", (0.0, 100.0), 50.0, true),
            ranged(
                "intensity",
                "Intensity",
                (0.0, 1000.0),
                (0.0, 400.0),
                100.0,
                true,
            ),
            colour("color", "Ray colour", Color([255, 242, 204, 255])),
            flag("only", "Rays only", false),
        ],
    },
    EffectDefinition {
        id: "light_leaks",
        name: "Light Leaks",
        shelf: EffectShelf::GlowLight,
        params: &[
            colour("c1", "Colour 1", Color([255, 115, 26, 255])),
            colour("c2", "Colour 2", Color([255, 38, 89, 255])),
            slider("intensity", "Intensity", (0.0, 100.0), 70.0, true),
            ranged("scale", "Scale", (10.0, 1000.0), (20.0, 400.0), 100.0, true),
            angle("direction", "Direction", 30.0),
            ranged("speed", "Speed", (0.0, 20.0), (0.0, 5.0), 1.0, true),
            ranged(
                "seed",
                "Random seed",
                (0.0, 10000.0),
                (0.0, 1000.0),
                0.0,
                true,
            ),
            choice("mode", "Blend mode", SCREEN_ADD_OVERLAY, 0),
        ],
    },
    EffectDefinition {
        id: "lens_flare",
        name: "Lens Flare",
        shelf: EffectShelf::GlowLight,
        params: &[
            point("center", "Flare centre", [0.5, 0.5]),
            ranged(
                "brightness",
                "Flare brightness",
                (0.0, 300.0),
                (0.0, 300.0),
                100.0,
                true,
            ),
            slider("blend", "Blend with original", (0.0, 100.0), 0.0, true),
        ],
    },
    EffectDefinition {
        id: "rgb_split",
        name: "Colour Split",
        shelf: EffectShelf::GlowLight,
        params: &[
            choice("mode", "Mode", SPLIT_MODES, 0),
            ranged("amount", "Amount", (0.0, 1000.0), (0.0, 100.0), 10.0, true),
            angle("angle", "Angle", 0.0),
            point("center", "Centre", [0.5, 0.5]),
            slider("blend", "Blend with original", (0.0, 100.0), 0.0, true),
        ],
    },
    // Stylize additions.
    EffectDefinition {
        id: "brush_strokes",
        name: "Paint Strokes",
        shelf: EffectShelf::Stylize,
        params: &[
            angle("angle", "Stroke angle", 135.0),
            ranged("size", "Brush size", (0.0, 5.0), (0.0, 5.0), 2.0, true),
            ranged(
                "length",
                "Stroke length",
                (0.0, 40.0),
                (0.0, 40.0),
                10.0,
                true,
            ),
            ranged(
                "density",
                "Stroke density",
                (0.0, 2.0),
                (0.0, 2.0),
                1.0,
                true,
            ),
        ],
    },
    EffectDefinition {
        id: "roughen_edges",
        name: "Torn Edges",
        shelf: EffectShelf::Stylize,
        params: &[
            choice("edge_type", "Edge type", TORN_EDGES, 0),
            ranged("border", "Border", (0.0, 500.0), (0.0, 100.0), 8.0, true),
            ranged("scale", "Scale", (10.0, 1000.0), (10.0, 400.0), 100.0, true),
            ranged(
                "complexity",
                "Complexity",
                (1.0, 10.0),
                (1.0, 10.0),
                2.0,
                true,
            ),
        ],
    },
    EffectDefinition {
        id: "camera_shake",
        name: "Hand-held Shake",
        shelf: EffectShelf::Stylize,
        params: &[
            ranged(
                "amount",
                "Position amount",
                (0.0, 1000.0),
                (0.0, 200.0),
                20.0,
                true,
            ),
            ranged(
                "rotation",
                "Rotation amount",
                (0.0, 45.0),
                (0.0, 10.0),
                1.0,
                true,
            ),
            slider("zoom", "Zoom amount", (0.0, 100.0), 5.0, true),
            ranged(
                "frequency",
                "Frequency",
                (0.1, 60.0),
                (0.1, 20.0),
                4.0,
                true,
            ),
        ],
    },
    EffectDefinition {
        id: "posterize_time",
        name: "Low Frame Rate",
        shelf: EffectShelf::Stylize,
        params: &[ranged(
            "rate",
            "Frame rate",
            (0.1, 99.0),
            (1.0, 60.0),
            12.0,
            true,
        )],
    },
    // Overlay & Key.
    EffectDefinition {
        id: "drop_shadow",
        name: "Drop Shadow",
        shelf: EffectShelf::OverlayKey,
        params: &[
            colour("color", "Shadow colour", BLACK),
            slider("opacity", "Opacity", (0.0, 100.0), 50.0, true),
            angle("direction", "Direction", 135.0),
            ranged(
                "distance",
                "Distance",
                (0.0, 4000.0),
                (0.0, 120.0),
                5.0,
                true,
            ),
            ranged(
                "softness",
                "Softness",
                (0.0, 1000.0),
                (0.0, 250.0),
                0.0,
                true,
            ),
            flag("only", "Shadow only", false),
        ],
    },
    EffectDefinition {
        id: "long_shadow",
        name: "Long Shadow",
        shelf: EffectShelf::OverlayKey,
        params: &[
            angle("angle", "Angle", 135.0),
            ranged("length", "Length", (0.0, 4000.0), (0.0, 600.0), 100.0, true),
            colour("color", "Shadow colour", BLACK),
            slider("opacity", "Opacity", (0.0, 100.0), 50.0, true),
            flag("fade", "Fade", true),
            flag("only", "Shadow only", false),
        ],
    },
    EffectDefinition {
        id: "gradient",
        name: "Gradient",
        shelf: EffectShelf::OverlayKey,
        params: &[
            point("start", "Gradient start", [0.5, 0.0]),
            colour("start_color", "Start colour", BLACK),
            point("end", "Gradient end", [0.5, 1.0]),
            colour("end_color", "End colour", WHITE),
            choice("shape", "Gradient shape", GRADIENT_SHAPES, 0),
        ],
    },
    EffectDefinition {
        id: "four_color_gradient",
        name: "Four-Corner Gradient",
        shelf: EffectShelf::OverlayKey,
        params: &[
            point("p1", "Point 1", [0.25, 0.25]),
            colour("c1", "Colour 1", Color([255, 255, 0, 255])),
            point("p2", "Point 2", [0.75, 0.25]),
            colour("c2", "Colour 2", Color([0, 255, 0, 255])),
            point("p3", "Point 3", [0.25, 0.75]),
            colour("c3", "Colour 3", Color([255, 0, 255, 255])),
            point("p4", "Point 4", [0.75, 0.75]),
            colour("c4", "Colour 4", Color([0, 0, 255, 255])),
        ],
    },
    EffectDefinition {
        id: "color_key",
        name: "Colour Key",
        shelf: EffectShelf::OverlayKey,
        params: &[
            colour("color", "Key colour", BLUE),
            slider("tolerance", "Colour tolerance", (0.0, 255.0), 0.0, true),
            slider("thin", "Edge thin", (-5.0, 5.0), 0.0, true),
            ranged(
                "feather",
                "Edge feather",
                (0.0, 50.0),
                (0.0, 50.0),
                0.0,
                true,
            ),
        ],
    },
    EffectDefinition {
        id: "luma_key",
        name: "Luma Key",
        shelf: EffectShelf::OverlayKey,
        params: &[
            slider("threshold", "Threshold", (0.0, 100.0), 0.0, true),
            slider("cutoff", "Cutoff", (0.0, 100.0), 0.0, true),
            flag("invert", "Key out brights", false),
        ],
    },
];

pub fn effect_definitions() -> &'static [EffectDefinition] {
    DEFINITIONS
}
