//! Built-in text looks: thirty style bundles over the curated OFL families, applied as values.
use crate::{Color, TextBackground, TextParams, TextShadow, TextStroke, TextStyle};

/// The text preset shelves in panel order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextShelf {
    Plain,
    Outline,
    Shadow,
    Glow,
    Label,
}

/// One built-in text look: its shelf, our own name and the style it copies onto text.
/// Content and animation keys never belong to a look.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltinTextPreset {
    shelf: TextShelf,
    name: &'static str,
    style: TextStyle,
}

impl BuiltinTextPreset {
    pub fn shelf(&self) -> TextShelf {
        self.shelf
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn style(&self) -> &TextStyle {
        &self.style
    }
}

/// Every built-in text look, shelves in panel order and entries in shelf order.
pub fn text_presets() -> Vec<BuiltinTextPreset> {
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

    let mut out = Vec::with_capacity(30);
    let mut push = |shelf: TextShelf, name: &'static str, style: TextStyle| {
        out.push(BuiltinTextPreset { shelf, name, style });
    };

    // Plain: fill only.
    push(
        TextShelf::Plain,
        "Clean White",
        style(BE_VIETNAM, REGULAR, 64.0, WHITE),
    );
    push(
        TextShelf::Plain,
        "Clean Bold",
        style(BE_VIETNAM, BOLD, 64.0, WHITE),
    );
    push(
        TextShelf::Plain,
        "Poster Anton",
        style(ANTON, REGULAR, 96.0, WHITE),
    );
    push(
        TextShelf::Plain,
        "Ink Black",
        style(BE_VIETNAM, REGULAR, 64.0, INK),
    );
    push(
        TextShelf::Plain,
        "Hand Note",
        style(PATRICK, REGULAR, 56.0, WHITE),
    );
    push(
        TextShelf::Plain,
        "Script Cream",
        style(PACIFICO, REGULAR, 64.0, [255, 244, 214, 255]),
    );

    // Outline: an outer stroke around the fill.
    push(
        TextShelf::Outline,
        "Outline Black",
        stroke(style(BE_VIETNAM, BOLD, 64.0, WHITE), BLACK, 4.0),
    );
    push(
        TextShelf::Outline,
        "Outline Yellow",
        stroke(style(ANTON, REGULAR, 96.0, GOLD), BLACK, 6.0),
    );
    push(
        TextShelf::Outline,
        "Outline Thin",
        stroke(style(BE_VIETNAM, REGULAR, 64.0, WHITE), BLACK, 2.0),
    );
    push(
        TextShelf::Outline,
        "Outline Navy",
        stroke(style(BE_VIETNAM, BOLD, 64.0, WHITE), [0, 32, 96, 255], 5.0),
    );
    push(
        TextShelf::Outline,
        "Outline Script",
        stroke(style(LOBSTER, REGULAR, 72.0, WHITE), [40, 20, 60, 255], 4.0),
    );
    push(
        TextShelf::Outline,
        "Outline Hand",
        stroke(
            style(PATRICK, REGULAR, 56.0, [255, 80, 80, 255]),
            WHITE,
            4.0,
        ),
    );

    // Shadow: a soft or hard offset copy of the glyphs behind the fill.
    push(
        TextShelf::Shadow,
        "Soft Shadow",
        shadow(
            style(BE_VIETNAM, REGULAR, 64.0, WHITE),
            [0, 0, 0, 160],
            2.0,
            3.0,
            4.0,
        ),
    );
    push(
        TextShelf::Shadow,
        "Hard Shadow",
        shadow(style(BE_VIETNAM, BOLD, 64.0, WHITE), BLACK, 4.0, 4.0, 0.0),
    );
    push(
        TextShelf::Shadow,
        "Long Shadow",
        shadow(
            style(ANTON, REGULAR, 96.0, WHITE),
            [0, 0, 0, 200],
            8.0,
            8.0,
            0.0,
        ),
    );
    push(
        TextShelf::Shadow,
        "Script Shadow",
        shadow(
            style(PACIFICO, REGULAR, 72.0, WHITE),
            [0, 0, 0, 180],
            3.0,
            4.0,
            6.0,
        ),
    );
    push(
        TextShelf::Shadow,
        "Hand Shadow",
        shadow(
            style(PATRICK, REGULAR, 56.0, GOLD),
            [0, 0, 0, 200],
            2.0,
            2.0,
            2.0,
        ),
    );

    // Glow: a soft halo in the fill colour.
    push(
        TextShelf::Glow,
        "Neon Cyan",
        glow(style(BE_VIETNAM, BOLD, 64.0, [0, 255, 255, 255])),
    );
    push(
        TextShelf::Glow,
        "Neon Pink",
        glow(style(LOBSTER, REGULAR, 72.0, [255, 64, 200, 255])),
    );
    push(
        TextShelf::Glow,
        "Glow Gold",
        glow(style(ANTON, REGULAR, 96.0, GOLD)),
    );
    push(
        TextShelf::Glow,
        "Glow White",
        glow(style(BE_VIETNAM, REGULAR, 64.0, WHITE)),
    );
    push(
        TextShelf::Glow,
        "Glow Mint",
        glow(style(PACIFICO, REGULAR, 72.0, [120, 255, 120, 255])),
    );

    // Label: a filled box behind the words.
    push(
        TextShelf::Label,
        "Label Black",
        boxed(
            style(BE_VIETNAM, REGULAR, 64.0, WHITE),
            [0, 0, 0, 200],
            16.0,
            8.0,
            8.0,
        ),
    );
    push(
        TextShelf::Label,
        "Label White",
        boxed(
            style(BE_VIETNAM, BOLD, 64.0, INK),
            [255, 255, 255, 230],
            16.0,
            8.0,
            8.0,
        ),
    );
    push(
        TextShelf::Label,
        "Label Yellow",
        boxed(style(BE_VIETNAM, BOLD, 64.0, INK), GOLD, 14.0, 8.0, 4.0),
    );
    push(
        TextShelf::Label,
        "Label Red",
        boxed(
            style(ANTON, REGULAR, 80.0, WHITE),
            [220, 38, 38, 255],
            18.0,
            6.0,
            4.0,
        ),
    );
    push(
        TextShelf::Label,
        "Label Blue",
        boxed(
            style(PATRICK, REGULAR, 56.0, WHITE),
            [30, 64, 175, 230],
            14.0,
            6.0,
            10.0,
        ),
    );
    push(
        TextShelf::Label,
        "Label Pill",
        boxed(
            style(BE_VIETNAM, BOLD, 64.0, WHITE),
            [0, 0, 0, 180],
            24.0,
            10.0,
            40.0,
        ),
    );
    push(
        TextShelf::Label,
        "Label Mint",
        boxed(
            style(PACIFICO, REGULAR, 64.0, [20, 40, 30, 255]),
            [167, 243, 208, 255],
            18.0,
            8.0,
            12.0,
        ),
    );
    push(
        TextShelf::Label,
        "Label Shifted",
        offset_box(
            style(BE_VIETNAM, REGULAR, 64.0, WHITE),
            [0, 0, 0, 200],
            14.0,
            8.0,
            6.0,
        ),
    );

    out
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

fn stroke(mut style: TextStyle, color: [u8; 4], width: f32) -> TextStyle {
    style.stroke = Some(TextStroke {
        color: Color(color),
        width,
    });
    style
}

fn shadow(
    mut style: TextStyle,
    color: [u8; 4],
    offset_x: f32,
    offset_y: f32,
    blur: f32,
) -> TextStyle {
    style.shadow = Some(TextShadow {
        color: Color(color),
        offset_x,
        offset_y,
        blur,
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
