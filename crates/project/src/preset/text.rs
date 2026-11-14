//! Built-in text looks: thirty style bundles over the curated OFL families, applied as values.
use crate::{Color, TextBackground, TextParams, TextShadow, TextStroke, TextStyle};

/// The text look shelves in panel order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextShelf {
    Plain,
    Outline,
    Shadow,
    Glow,
    Label,
}

/// A built-in text look, by stable snake_case wire id. Applying one copies its style onto
/// text items and keeps every item's content and animation keys.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TextLook {
    CleanWhite,
    CleanBold,
    PosterAnton,
    InkBlack,
    HandNote,
    ScriptCream,
    OutlineBlack,
    OutlineYellow,
    OutlineThin,
    OutlineNavy,
    OutlineScript,
    OutlineHand,
    SoftShadow,
    HardShadow,
    LongShadow,
    ScriptShadow,
    HandShadow,
    NeonCyan,
    NeonPink,
    GlowGold,
    GlowWhite,
    GlowMint,
    LabelBlack,
    LabelWhite,
    LabelYellow,
    LabelRed,
    LabelBlue,
    LabelPill,
    LabelMint,
    LabelShifted,
}

impl TextLook {
    /// Every look, shelves in panel order and entries in shelf order.
    pub const ALL: [Self; 30] = [
        Self::CleanWhite,
        Self::CleanBold,
        Self::PosterAnton,
        Self::InkBlack,
        Self::HandNote,
        Self::ScriptCream,
        Self::OutlineBlack,
        Self::OutlineYellow,
        Self::OutlineThin,
        Self::OutlineNavy,
        Self::OutlineScript,
        Self::OutlineHand,
        Self::SoftShadow,
        Self::HardShadow,
        Self::LongShadow,
        Self::ScriptShadow,
        Self::HandShadow,
        Self::NeonCyan,
        Self::NeonPink,
        Self::GlowGold,
        Self::GlowWhite,
        Self::GlowMint,
        Self::LabelBlack,
        Self::LabelWhite,
        Self::LabelYellow,
        Self::LabelRed,
        Self::LabelBlue,
        Self::LabelPill,
        Self::LabelMint,
        Self::LabelShifted,
    ];

    /// The shelf the panel groups this look under.
    pub const fn shelf(self) -> TextShelf {
        match self {
            Self::CleanWhite
            | Self::CleanBold
            | Self::PosterAnton
            | Self::InkBlack
            | Self::HandNote
            | Self::ScriptCream => TextShelf::Plain,
            Self::OutlineBlack
            | Self::OutlineYellow
            | Self::OutlineThin
            | Self::OutlineNavy
            | Self::OutlineScript
            | Self::OutlineHand => TextShelf::Outline,
            Self::SoftShadow
            | Self::HardShadow
            | Self::LongShadow
            | Self::ScriptShadow
            | Self::HandShadow => TextShelf::Shadow,
            Self::NeonCyan | Self::NeonPink | Self::GlowGold | Self::GlowWhite | Self::GlowMint => {
                TextShelf::Glow
            }
            Self::LabelBlack
            | Self::LabelWhite
            | Self::LabelYellow
            | Self::LabelRed
            | Self::LabelBlue
            | Self::LabelPill
            | Self::LabelMint
            | Self::LabelShifted => TextShelf::Label,
        }
    }

    /// Our own display name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::CleanWhite => "Clean White",
            Self::CleanBold => "Clean Bold",
            Self::PosterAnton => "Poster Anton",
            Self::InkBlack => "Ink Black",
            Self::HandNote => "Hand Note",
            Self::ScriptCream => "Script Cream",
            Self::OutlineBlack => "Outline Black",
            Self::OutlineYellow => "Outline Yellow",
            Self::OutlineThin => "Outline Thin",
            Self::OutlineNavy => "Outline Navy",
            Self::OutlineScript => "Outline Script",
            Self::OutlineHand => "Outline Hand",
            Self::SoftShadow => "Soft Shadow",
            Self::HardShadow => "Hard Shadow",
            Self::LongShadow => "Long Shadow",
            Self::ScriptShadow => "Script Shadow",
            Self::HandShadow => "Hand Shadow",
            Self::NeonCyan => "Neon Cyan",
            Self::NeonPink => "Neon Pink",
            Self::GlowGold => "Glow Gold",
            Self::GlowWhite => "Glow White",
            Self::GlowMint => "Glow Mint",
            Self::LabelBlack => "Label Black",
            Self::LabelWhite => "Label White",
            Self::LabelYellow => "Label Yellow",
            Self::LabelRed => "Label Red",
            Self::LabelBlue => "Label Blue",
            Self::LabelPill => "Label Pill",
            Self::LabelMint => "Label Mint",
            Self::LabelShifted => "Label Shifted",
        }
    }

    /// The style this look copies onto text: everything except content and keys.
    pub fn style(self) -> TextStyle {
        const WHITE: [u8; 4] = [255, 255, 255, 255];
        const BLACK: [u8; 4] = [0, 0, 0, 255];
        const GOLD: [u8; 4] = [255, 215, 0, 255];
        const INK: [u8; 4] = [17, 17, 17, 255];
        const BE_VIETNAM: &str = "Be Vietnam Pro";
        const ANTON: &str = "Anton";
        const LOBSTER: &str = "Lobster";
        const PACIFICO: &str = "Pacifico";
        const PATRICK: &str = "Patrick Hand";
        const REGULAR: u16 = 400;
        const BOLD: u16 = 700;
        match self {
            Self::CleanWhite => style(BE_VIETNAM, REGULAR, 64.0, WHITE),
            Self::CleanBold => style(BE_VIETNAM, BOLD, 64.0, WHITE),
            Self::PosterAnton => style(ANTON, REGULAR, 96.0, WHITE),
            Self::InkBlack => style(BE_VIETNAM, REGULAR, 64.0, INK),
            Self::HandNote => style(PATRICK, REGULAR, 56.0, WHITE),
            Self::ScriptCream => style(PACIFICO, REGULAR, 64.0, [255, 244, 214, 255]),
            Self::OutlineBlack => stroke(style(BE_VIETNAM, BOLD, 64.0, WHITE), BLACK, 30.0),
            Self::OutlineYellow => stroke(style(ANTON, REGULAR, 96.0, GOLD), BLACK, 30.0),
            Self::OutlineThin => stroke(style(BE_VIETNAM, REGULAR, 64.0, WHITE), BLACK, 15.0),
            Self::OutlineNavy => {
                stroke(style(BE_VIETNAM, BOLD, 64.0, WHITE), [0, 32, 96, 255], 40.0)
            }
            Self::OutlineScript => stroke(
                style(LOBSTER, REGULAR, 72.0, WHITE),
                [40, 20, 60, 255],
                30.0,
            ),
            Self::OutlineHand => stroke(
                style(PATRICK, REGULAR, 56.0, [255, 80, 80, 255]),
                WHITE,
                35.0,
            ),
            Self::SoftShadow => shadow(
                style(BE_VIETNAM, REGULAR, 64.0, WHITE),
                BLACK,
                63.0,
                25.0,
                6.0,
                -55.0,
            ),
            Self::HardShadow => shadow(
                style(BE_VIETNAM, BOLD, 64.0, WHITE),
                BLACK,
                100.0,
                0.0,
                9.0,
                -45.0,
            ),
            Self::LongShadow => shadow(
                style(ANTON, REGULAR, 96.0, WHITE),
                BLACK,
                78.0,
                0.0,
                12.0,
                -45.0,
            ),
            Self::ScriptShadow => shadow(
                style(PACIFICO, REGULAR, 72.0, WHITE),
                BLACK,
                71.0,
                33.0,
                7.0,
                -53.0,
            ),
            Self::HandShadow => shadow(
                style(PATRICK, REGULAR, 56.0, GOLD),
                BLACK,
                78.0,
                14.0,
                5.0,
                -45.0,
            ),
            Self::NeonCyan => glow(style(BE_VIETNAM, BOLD, 64.0, [0, 255, 255, 255])),
            Self::NeonPink => glow(style(LOBSTER, REGULAR, 72.0, [255, 64, 200, 255])),
            Self::GlowGold => glow(style(ANTON, REGULAR, 96.0, GOLD)),
            Self::GlowWhite => glow(style(BE_VIETNAM, REGULAR, 64.0, WHITE)),
            Self::GlowMint => glow(style(PACIFICO, REGULAR, 72.0, [120, 255, 120, 255])),
            Self::LabelBlack => boxed(
                style(BE_VIETNAM, REGULAR, 64.0, WHITE),
                [0, 0, 0, 200],
                16.0,
                8.0,
                8.0,
            ),
            Self::LabelWhite => boxed(
                style(BE_VIETNAM, BOLD, 64.0, INK),
                [255, 255, 255, 230],
                16.0,
                8.0,
                8.0,
            ),
            Self::LabelYellow => boxed(style(BE_VIETNAM, BOLD, 64.0, INK), GOLD, 14.0, 8.0, 4.0),
            Self::LabelRed => boxed(
                style(ANTON, REGULAR, 80.0, WHITE),
                [220, 38, 38, 255],
                18.0,
                6.0,
                4.0,
            ),
            Self::LabelBlue => boxed(
                style(PATRICK, REGULAR, 56.0, WHITE),
                [30, 64, 175, 230],
                14.0,
                6.0,
                10.0,
            ),
            Self::LabelPill => boxed(
                style(BE_VIETNAM, BOLD, 64.0, WHITE),
                [0, 0, 0, 180],
                24.0,
                10.0,
                40.0,
            ),
            Self::LabelMint => boxed(
                style(PACIFICO, REGULAR, 64.0, [20, 40, 30, 255]),
                [167, 243, 208, 255],
                18.0,
                8.0,
                12.0,
            ),
            Self::LabelShifted => offset_box(
                style(BE_VIETNAM, REGULAR, 64.0, WHITE),
                [0, 0, 0, 200],
                14.0,
                8.0,
                6.0,
            ),
        }
    }
}

/// A text style with the given family, weight, size and fill; every other field at its default.
fn style(font: &str, weight: u16, size: f32, color: [u8; 4]) -> TextStyle {
    let mut params = TextParams::new("");
    params.font = font.into();
    params.weight = weight;
    params.font_size = size;
    params.color = Color(color);
    TextStyle::from_params(&params)
}

fn stroke(mut style: TextStyle, color: [u8; 4], thickness: f32) -> TextStyle {
    style.stroke = Some(TextStroke {
        color: Color(color),
        thickness,
    });
    style
}

fn shadow(
    mut style: TextStyle,
    color: [u8; 4],
    opacity: f32,
    blurriness: f32,
    distance: f32,
    angle: f32,
) -> TextStyle {
    style.shadow = Some(TextShadow {
        color: Color(color),
        opacity,
        blurriness,
        distance,
        angle,
    });
    style
}

fn glow(mut style: TextStyle) -> TextStyle {
    style.glow = true;
    style
}

fn boxed(
    mut style: TextStyle,
    color: [u8; 4],
    padding_x: f32,
    padding_y: f32,
    radius: f32,
) -> TextStyle {
    style.background = Some(TextBackground {
        color: Color(color),
        padding_x,
        padding_y,
        offset_x: 0.0,
        offset_y: 0.0,
        radius,
    });
    style
}

fn offset_box(
    mut style: TextStyle,
    color: [u8; 4],
    padding: f32,
    offset_y: f32,
    radius: f32,
) -> TextStyle {
    style.background = Some(TextBackground {
        color: Color(color),
        padding_x: padding,
        padding_y: padding,
        offset_x: 0.0,
        offset_y,
        radius,
    });
    style
}
