//! The admitted transition catalogue shared by command validation, rendering and the browser.
use super::transition_params::{TransitionParamDef, TransitionParamKind};
use crate::{Color, Error, Result};

/// One admitted kind: stable id, our display name, grouping and typed parameters.
#[derive(Clone, Copy, Debug)]
pub struct TransitionDefinition {
    pub kind: &'static str,
    pub name: &'static str,
    pub family: &'static str,
    /// The browser shelf that lists the entry.
    pub shelf: &'static str,
    pub params: &'static [TransitionParamDef],
}
impl TransitionDefinition {
    pub fn param(&self, id: &str) -> Option<&'static TransitionParamDef> {
        self.params.iter().find(|p| p.id == id)
    }
}

const DIP_COLOR: TransitionParamDef = TransitionParamDef {
    id: "color",
    name: "Color",
    kind: TransitionParamKind::Color {
        default: Color([217, 51, 51, 255]),
    },
};
const DIP_HOLD: TransitionParamDef = TransitionParamDef {
    id: "hold",
    name: "Hold",
    kind: TransitionParamKind::Float {
        min: 0.,
        max: 90.,
        default: 0.,
        unit: "%",
    },
};

const fn percent(id: &'static str, name: &'static str, default: f64) -> TransitionParamDef {
    TransitionParamDef {
        id,
        name,
        kind: TransitionParamKind::Float {
            min: 0.,
            max: 100.,
            default,
            unit: "%",
        },
    }
}
const fn pixels(
    id: &'static str,
    name: &'static str,
    default: f64,
    max: f64,
) -> TransitionParamDef {
    TransitionParamDef {
        id,
        name,
        kind: TransitionParamKind::Float {
            min: 0.,
            max,
            default,
            unit: "px",
        },
    }
}
const fn ranged(
    id: &'static str,
    name: &'static str,
    default: f64,
    min: f64,
    max: f64,
) -> TransitionParamDef {
    TransitionParamDef {
        id,
        name,
        kind: TransitionParamKind::Float {
            min,
            max,
            default,
            unit: "",
        },
    }
}
const fn colour(id: &'static str, name: &'static str, rgb: [u8; 3]) -> TransitionParamDef {
    TransitionParamDef {
        id,
        name,
        kind: TransitionParamKind::Color {
            default: Color([rgb[0], rgb[1], rgb[2], 255]),
        },
    }
}
const fn angle(id: &'static str, name: &'static str, default: f64) -> TransitionParamDef {
    TransitionParamDef {
        id,
        name,
        kind: TransitionParamKind::Angle { default },
    }
}
const fn choice(
    id: &'static str,
    name: &'static str,
    options: &'static [&'static str],
    default: u32,
) -> TransitionParamDef {
    TransitionParamDef {
        id,
        name,
        kind: TransitionParamKind::Choice { options, default },
    }
}
const fn flag(id: &'static str, name: &'static str, default: bool) -> TransitionParamDef {
    TransitionParamDef {
        id,
        name,
        kind: TransitionParamKind::Bool { default },
    }
}
const fn point(id: &'static str, name: &'static str) -> TransitionParamDef {
    TransitionParamDef {
        id,
        name,
        kind: TransitionParamKind::Point {
            default: [0.5, 0.5],
        },
    }
}
const fn seed() -> TransitionParamDef {
    TransitionParamDef {
        id: "seed",
        name: "Seed",
        kind: TransitionParamKind::Seed { default: 1 },
    }
}

const DIRECTIONS: &[&str] = &["North", "East", "South", "West"];
const AXES: &[&str] = &["Horizontal", "Vertical"];
const CORNERS: &[&str] = &["Top Left", "Top Right", "Bottom Right", "Bottom Left"];
const AA: &[&str] = &["Off", "Low", "Medium", "High"];
const SOURCES: &[&str] = &["Outgoing", "Incoming"];

const CENTRE: TransitionParamDef = point("center", "Centre");
const DIRECTION: TransitionParamDef = choice("direction", "Direction", DIRECTIONS, 3);
const FEATHER: TransitionParamDef = pixels("feather", "Feather", 0., 500.);
const BORDER_WIDTH: TransitionParamDef = pixels("border_width", "Border Width", 0., 200.);
const BORDER_COLOR: TransitionParamDef = colour("border_color", "Border Colour", [0, 0, 0]);
const ANTIALIAS: TransitionParamDef = choice("antialias", "Anti-aliasing", AA, 1);
const AMOUNT: TransitionParamDef = percent("amount", "Amount", 80.);

pub const TRANSITION_DEFINITIONS: &[TransitionDefinition] = &[
    TransitionDefinition {
        kind: "cross_dissolve",
        name: "Cross Dissolve",
        family: "Dissolve",
        shelf: "Basic",
        params: &[],
    },
    TransitionDefinition {
        kind: "dip_to_black",
        name: "Dip to Black",
        family: "Dissolve",
        shelf: "Basic",
        params: &[],
    },
    TransitionDefinition {
        kind: "dip_to_white",
        name: "Dip to White",
        family: "Dissolve",
        shelf: "Basic",
        params: &[],
    },
    TransitionDefinition {
        kind: "dip_to_color",
        name: "Dip to Colour",
        family: "Dissolve",
        shelf: "Basic",
        params: &[DIP_COLOR, DIP_HOLD],
    },
    // ---- Dissolves (261) ----
    TransitionDefinition {
        kind: "light_fade",
        name: "Light Fade",
        family: "Dissolve",
        shelf: "Basic",
        params: &[],
    },
    TransitionDefinition {
        kind: "film_fade",
        name: "Film Fade",
        family: "Dissolve",
        shelf: "Basic",
        params: &[],
    },
    TransitionDefinition {
        kind: "soft_focus_fade",
        name: "Soft Focus Fade",
        family: "Dissolve",
        shelf: "Blur",
        params: &[pixels("blur", "Blur", 40., 400.)],
    },
    TransitionDefinition {
        kind: "burn_through",
        name: "Burn Through",
        family: "Dissolve",
        shelf: "Overlay",
        params: &[
            colour("burn_color", "Burn Colour", [255, 115, 26]),
            percent("softness", "Softness", 25.),
            seed(),
        ],
    },
    TransitionDefinition {
        kind: "brightness_fade",
        name: "Brightness Fade",
        family: "Dissolve",
        shelf: "Basic",
        params: &[
            choice("source", "Brightness Source", SOURCES, 0),
            percent("softness", "Softness", 20.),
            flag("invert", "Invert", false),
        ],
    },
    TransitionDefinition {
        kind: "pixel_blocks",
        name: "Pixel Blocks",
        family: "Dissolve",
        shelf: "Basic",
        params: &[
            pixels("block_size", "Block Size", 64., 400.),
            flag("dissolve", "Fade Blocks", true),
        ],
    },
    // ---- Wipes (261) ----
    TransitionDefinition {
        kind: "straight_wipe",
        name: "Straight",
        family: "Wipe",
        shelf: "Wipes",
        params: &[
            angle("angle", "Angle", 90.),
            FEATHER,
            BORDER_WIDTH,
            BORDER_COLOR,
            ANTIALIAS,
        ],
    },
    TransitionDefinition {
        kind: "soft_edge_wipe",
        name: "Soft Edge",
        family: "Wipe",
        shelf: "Wipes",
        params: &[
            angle("angle", "Angle", 90.),
            pixels("feather", "Feather", 300., 500.),
            BORDER_WIDTH,
            BORDER_COLOR,
            ANTIALIAS,
        ],
    },
    TransitionDefinition {
        kind: "clock_wipe",
        name: "Clock",
        family: "Wipe",
        shelf: "Wipes",
        params: &[
            CENTRE,
            angle("start", "Start Angle", 0.),
            FEATHER,
            BORDER_WIDTH,
            BORDER_COLOR,
            ANTIALIAS,
        ],
    },
    TransitionDefinition {
        kind: "corner_sweep",
        name: "Corner Sweep",
        family: "Wipe",
        shelf: "Wipes",
        params: &[
            choice("corner", "Corner", CORNERS, 0),
            FEATHER,
            BORDER_WIDTH,
            BORDER_COLOR,
            ANTIALIAS,
        ],
    },
    TransitionDefinition {
        kind: "star_wipe",
        name: "Star",
        family: "Wipe",
        shelf: "Wipes",
        params: &[
            CENTRE,
            ranged("points", "Points", 5., 3., 24.),
            TransitionParamDef {
                id: "inner",
                name: "Inner Radius",
                kind: TransitionParamKind::Float {
                    min: 5.,
                    max: 100.,
                    default: 45.,
                    unit: "%",
                },
            },
            angle("rotation", "Rotation", 0.),
            FEATHER,
            BORDER_WIDTH,
            BORDER_COLOR,
            ANTIALIAS,
        ],
    },
    TransitionDefinition {
        kind: "panel_wipe",
        name: "Panel",
        family: "Wipe",
        shelf: "Wipes",
        params: &[
            DIRECTION,
            ranged("panels", "Panels", 6., 1., 64.),
            FEATHER,
            BORDER_WIDTH,
            BORDER_COLOR,
            ANTIALIAS,
        ],
    },
    TransitionDefinition {
        kind: "stair_wipe",
        name: "Stair",
        family: "Wipe",
        shelf: "Wipes",
        params: &[
            angle("angle", "Angle", 90.),
            ranged("plateaus", "Steps", 5., 1., 64.),
            pixels("feather", "Feather", 4., 500.),
            BORDER_WIDTH,
            BORDER_COLOR,
            ANTIALIAS,
        ],
    },
    TransitionDefinition {
        kind: "stretch_wipe",
        name: "Stretch",
        family: "Wipe",
        shelf: "Wipes",
        params: &[DIRECTION, pixels("stretch", "Stretch", 200., 2000.)],
    },
    TransitionDefinition {
        kind: "neon_edge_wipe",
        name: "Neon Edge",
        family: "Wipe",
        shelf: "Wipes",
        params: &[
            angle("angle", "Angle", 90.),
            colour("color", "Neon Colour", [51, 229, 255]),
            pixels("glow", "Glow Width", 40., 400.),
            percent("amount", "Amount", 100.),
        ],
    },
    TransitionDefinition {
        kind: "circle_reveal",
        name: "Circle Reveal",
        family: "Wipe",
        shelf: "Wipes",
        params: &[CENTRE, FEATHER, BORDER_WIDTH, BORDER_COLOR, ANTIALIAS],
    },
    TransitionDefinition {
        kind: "box_reveal",
        name: "Box Reveal",
        family: "Wipe",
        shelf: "Wipes",
        params: &[CENTRE, FEATHER, BORDER_WIDTH, BORDER_COLOR, ANTIALIAS],
    },
    TransitionDefinition {
        kind: "diamond_reveal",
        name: "Diamond Reveal",
        family: "Wipe",
        shelf: "Wipes",
        params: &[CENTRE, FEATHER, BORDER_WIDTH, BORDER_COLOR, ANTIALIAS],
    },
    TransitionDefinition {
        kind: "cross_reveal",
        name: "Cross Reveal",
        family: "Wipe",
        shelf: "Wipes",
        params: &[CENTRE, FEATHER, BORDER_WIDTH, BORDER_COLOR, ANTIALIAS],
    },
    TransitionDefinition {
        kind: "double_doors",
        name: "Double Doors",
        family: "Wipe",
        shelf: "Wipes",
        params: &[
            choice("orientation", "Orientation", AXES, 1),
            FEATHER,
            BORDER_WIDTH,
            BORDER_COLOR,
            ANTIALIAS,
        ],
    },
    TransitionDefinition {
        kind: "corner_inset",
        name: "Corner Inset",
        family: "Wipe",
        shelf: "Wipes",
        params: &[
            choice("corner", "Corner", CORNERS, 0),
            FEATHER,
            BORDER_WIDTH,
            BORDER_COLOR,
            ANTIALIAS,
        ],
    },
    // ---- Lights and blurs (261) ----
    TransitionDefinition {
        kind: "zoom_cross",
        name: "Zoom Cross",
        family: "Lights & Blurs",
        shelf: "Light",
        params: &[CENTRE, percent("strength", "Zoom Strength", 60.)],
    },
    TransitionDefinition {
        kind: "zoom_smear",
        name: "Zoom Smear",
        family: "Lights & Blurs",
        shelf: "Blur",
        params: &[CENTRE, percent("strength", "Strength", 50.)],
    },
    TransitionDefinition {
        kind: "spin_smear",
        name: "Spin Smear",
        family: "Lights & Blurs",
        shelf: "Blur",
        params: &[
            CENTRE,
            TransitionParamDef {
                id: "angle",
                name: "Blur Angle",
                kind: TransitionParamKind::Angle { default: 40. },
            },
        ],
    },
    TransitionDefinition {
        kind: "streak_smear",
        name: "Streak Smear",
        family: "Lights & Blurs",
        shelf: "Blur",
        params: &[
            angle("angle", "Angle", 90.),
            pixels("blur", "Blur Length", 160., 1000.),
        ],
    },
    TransitionDefinition {
        kind: "lens_defocus",
        name: "Lens Defocus",
        family: "Lights & Blurs",
        shelf: "Blur",
        params: &[pixels("radius", "Iris Radius", 30., 200.)],
    },
    TransitionDefinition {
        kind: "bloom",
        name: "Bloom",
        family: "Lights & Blurs",
        shelf: "Light",
        params: &[
            pixels("radius", "Radius", 40., 400.),
            percent("intensity", "Intensity", 80.),
        ],
    },
    TransitionDefinition {
        kind: "flash",
        name: "Flash",
        family: "Lights & Blurs",
        shelf: "Overlay",
        params: &[
            colour("color", "Colour", [255, 255, 255]),
            percent("intensity", "Intensity", 100.),
        ],
    },
    TransitionDefinition {
        kind: "lens_flare",
        name: "Lens Flare",
        family: "Lights & Blurs",
        shelf: "Light",
        params: &[
            colour("color", "Colour", [255, 204, 140]),
            AMOUNT,
            angle("angle", "Angle", 20.),
        ],
    },
    TransitionDefinition {
        kind: "light_leak",
        name: "Light Leak",
        family: "Lights & Blurs",
        shelf: "Overlay",
        params: &[colour("color", "Colour", [255, 140, 51]), AMOUNT, seed()],
    },
    TransitionDefinition {
        kind: "colour_leak",
        name: "Colour Leak",
        family: "Lights & Blurs",
        shelf: "Overlay",
        params: &[percent("amount", "Amount", 70.), seed()],
    },
    TransitionDefinition {
        kind: "colour_burn",
        name: "Colour Burn",
        family: "Lights & Blurs",
        shelf: "Overlay",
        params: &[AMOUNT],
    },
    TransitionDefinition {
        kind: "light_sweep",
        name: "Light Sweep",
        family: "Lights & Blurs",
        shelf: "Light",
        params: &[
            angle("angle", "Angle", 70.),
            pixels("width", "Width", 160., 1000.),
            colour("color", "Colour", [255, 247, 230]),
            AMOUNT,
        ],
    },
    TransitionDefinition {
        kind: "light_rays",
        name: "Light Rays",
        family: "Lights & Blurs",
        shelf: "Light",
        params: &[CENTRE, AMOUNT, percent("length", "Ray Length", 50.)],
    },
    TransitionDefinition {
        kind: "phosphor_trail",
        name: "Phosphor Trail",
        family: "Lights & Blurs",
        shelf: "Overlay",
        params: &[
            colour("color", "Phosphor Colour", [140, 255, 153]),
            percent("decay", "Decay", 50.),
        ],
    },
    TransitionDefinition {
        kind: "solarise",
        name: "Solarise",
        family: "Lights & Blurs",
        shelf: "Overlay",
        params: &[percent("amount", "Amount", 100.)],
    },
    TransitionDefinition {
        kind: "stripes",
        name: "Stripes",
        family: "Lights & Blurs",
        shelf: "Light",
        params: &[
            ranged("stripes", "Stripes", 8., 1., 64.),
            angle("angle", "Angle", 30.),
            colour("color", "Colour", [255, 255, 255]),
        ],
    },
    // ---- Grunge and distort (261) ----
    TransitionDefinition {
        kind: "scramble",
        name: "Scramble",
        family: "Grunge & Distort",
        shelf: "Glitch & Distort",
        params: &[percent("amount", "Amount", 60.), seed()],
    },
    TransitionDefinition {
        kind: "digital_glitch",
        name: "Digital Glitch",
        family: "Grunge & Distort",
        shelf: "Glitch & Distort",
        params: &[percent("amount", "Amount", 60.), seed()],
    },
    TransitionDefinition {
        kind: "tape_damage",
        name: "Tape Damage",
        family: "Grunge & Distort",
        shelf: "Glitch & Distort",
        params: &[percent("amount", "Amount", 60.), seed()],
    },
    TransitionDefinition {
        kind: "flicker_cut",
        name: "Flicker Cut",
        family: "Grunge & Distort",
        shelf: "Glitch & Distort",
        params: &[ranged("flickers", "Flickers", 8., 1., 40.), seed()],
    },
    TransitionDefinition {
        kind: "quake",
        name: "Quake",
        family: "Grunge & Distort",
        shelf: "Glitch & Distort",
        params: &[
            pixels("shake", "Shake", 40., 400.),
            seed(),
            percent("motion_blur", "Motion Blur", 50.),
        ],
    },
    TransitionDefinition {
        kind: "glass_shards",
        name: "Glass Shards",
        family: "Grunge & Distort",
        shelf: "Glitch & Distort",
        params: &[
            pixels("cell_size", "Shard Size", 90., 1000.),
            pixels("refraction", "Refraction", 40., 400.),
            seed(),
        ],
    },
    TransitionDefinition {
        kind: "rough_edge",
        name: "Rough Edge",
        family: "Grunge & Distort",
        shelf: "Glitch & Distort",
        params: &[
            pixels("scale", "Scale", 120., 1000.),
            colour("edge_color", "Edge Colour", [56, 36, 20]),
            pixels("edge_width", "Edge Width", 12., 200.),
            seed(),
        ],
    },
    TransitionDefinition {
        kind: "kaleidoscope",
        name: "Kaleidoscope",
        family: "Grunge & Distort",
        shelf: "Glitch & Distort",
        params: &[
            ranged("segments", "Segments", 6., 2., 32.),
            percent("zoom", "Zoom", 30.),
        ],
    },
    TransitionDefinition {
        kind: "liquid_warp",
        name: "Liquid Warp",
        family: "Grunge & Distort",
        shelf: "Glitch & Distort",
        params: &[
            pixels("amount", "Amount", 60., 500.),
            pixels("scale", "Scale", 200., 2000.),
            seed(),
        ],
    },
    TransitionDefinition {
        kind: "screen_off",
        name: "Screen Off",
        family: "Grunge & Distort",
        shelf: "Glitch & Distort",
        params: &[
            colour("glow_color", "Glow Colour", [217, 242, 255]),
            pixels("line", "Line Thickness", 4., 50.),
        ],
    },
];

/// Look up an admitted kind; unsupported names return a typed error capped at 64 characters.
pub fn transition_definition(kind: &str) -> Result<&'static TransitionDefinition> {
    TRANSITION_DEFINITIONS
        .iter()
        .find(|d| d.kind == kind)
        .ok_or_else(|| Error::UnsupportedTransition(kind.chars().take(64).collect()))
}
