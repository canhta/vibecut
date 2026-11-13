//! Built-in caption looks: thirty complete caption styles over the curated OFL families.
use captions::{CaptionBackground, CaptionOutline, CaptionPosition, CaptionStyle};

/// The caption preset shelves in panel order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptionShelf {
    Plain,
    Outline,
    Label,
    Karaoke,
}

/// One built-in caption look: its shelf, our own name and the complete caption style it sets.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltinCaptionPreset {
    shelf: CaptionShelf,
    name: &'static str,
    style: CaptionStyle,
}

impl BuiltinCaptionPreset {
    pub fn shelf(&self) -> CaptionShelf {
        self.shelf
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn style(&self) -> &CaptionStyle {
        &self.style
    }
}

/// Every built-in caption look, shelves in panel order and entries in shelf order.
pub fn caption_presets() -> Vec<BuiltinCaptionPreset> {
    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const BLACK: [u8; 4] = [0, 0, 0, 255];
    const GOLD: [u8; 3] = [255, 215, 0];
    const INK: [u8; 4] = [17, 17, 17, 255];
    const BE_VIETNAM: &str = "Be Vietnam Pro";
    const ANTON: &str = "Anton";
    const PACIFICO: &str = "Pacifico";
    const PATRICK: &str = "Patrick Hand";
    const REGULAR: &str = "Regular";
    const BOLD: &str = "Bold";

    let mut out = Vec::with_capacity(30);
    let mut push = |shelf: CaptionShelf, name: &'static str, style: CaptionStyle| {
        out.push(BuiltinCaptionPreset { shelf, name, style });
    };

    // Plain: fill only, no outline and no box.
    push(
        CaptionShelf::Plain,
        "Plain Bottom",
        style(BE_VIETNAM, REGULAR, 48.0, WHITE),
    );
    push(
        CaptionShelf::Plain,
        "Plain Middle",
        at(
            style(BE_VIETNAM, REGULAR, 48.0, WHITE),
            CaptionPosition::Middle,
        ),
    );
    push(
        CaptionShelf::Plain,
        "Plain Top",
        at(
            style(BE_VIETNAM, REGULAR, 48.0, WHITE),
            CaptionPosition::Top,
        ),
    );
    push(
        CaptionShelf::Plain,
        "Plain Bold",
        style(BE_VIETNAM, BOLD, 52.0, WHITE),
    );
    push(
        CaptionShelf::Plain,
        "Plain Large",
        one_line(style(BE_VIETNAM, BOLD, 64.0, WHITE)),
    );
    push(
        CaptionShelf::Plain,
        "Plain Ink",
        style(BE_VIETNAM, REGULAR, 44.0, INK),
    );
    push(
        CaptionShelf::Plain,
        "Plain Cream",
        style(PACIFICO, REGULAR, 48.0, [255, 244, 214, 255]),
    );
    push(
        CaptionShelf::Plain,
        "Plain Poster",
        style(ANTON, REGULAR, 56.0, WHITE),
    );

    // Outline: a stroke around every glyph.
    push(
        CaptionShelf::Outline,
        "Outline Bottom",
        outlined(style(BE_VIETNAM, BOLD, 48.0, WHITE), BLACK, 3.0),
    );
    push(
        CaptionShelf::Outline,
        "Outline Thin",
        outlined(style(BE_VIETNAM, REGULAR, 44.0, WHITE), BLACK, 2.0),
    );
    push(
        CaptionShelf::Outline,
        "Outline Thick",
        outlined(style(BE_VIETNAM, BOLD, 52.0, WHITE), BLACK, 5.0),
    );
    push(
        CaptionShelf::Outline,
        "Outline Yellow",
        outlined(
            style(BE_VIETNAM, BOLD, 48.0, [255, 215, 0, 255]),
            BLACK,
            3.0,
        ),
    );
    push(
        CaptionShelf::Outline,
        "Outline Top",
        at(
            outlined(style(BE_VIETNAM, BOLD, 48.0, WHITE), BLACK, 3.0),
            CaptionPosition::Top,
        ),
    );
    push(
        CaptionShelf::Outline,
        "Outline Middle",
        at(
            outlined(style(BE_VIETNAM, BOLD, 48.0, WHITE), BLACK, 3.0),
            CaptionPosition::Middle,
        ),
    );
    push(
        CaptionShelf::Outline,
        "Outline Poster",
        outlined(style(ANTON, REGULAR, 60.0, WHITE), BLACK, 4.0),
    );
    push(
        CaptionShelf::Outline,
        "Outline Hand",
        outlined(style(PATRICK, REGULAR, 52.0, WHITE), [20, 20, 60, 255], 3.0),
    );

    // Label: a filled box behind the caption lines.
    push(
        CaptionShelf::Label,
        "Label Black",
        boxed(
            style(BE_VIETNAM, BOLD, 48.0, WHITE),
            [0, 0, 0, 160],
            12.0,
            6.0,
        ),
    );
    push(
        CaptionShelf::Label,
        "Label White",
        boxed(
            style(BE_VIETNAM, BOLD, 48.0, INK),
            [255, 255, 255, 230],
            12.0,
            6.0,
        ),
    );
    push(
        CaptionShelf::Label,
        "Label Yellow",
        boxed(
            style(BE_VIETNAM, BOLD, 48.0, INK),
            [255, 215, 0, 255],
            12.0,
            6.0,
        ),
    );
    push(
        CaptionShelf::Label,
        "Label Pill",
        boxed(
            style(BE_VIETNAM, BOLD, 46.0, WHITE),
            [0, 0, 0, 180],
            18.0,
            24.0,
        ),
    );
    push(
        CaptionShelf::Label,
        "Label Red",
        boxed(
            style(ANTON, REGULAR, 56.0, WHITE),
            [220, 38, 38, 255],
            14.0,
            4.0,
        ),
    );
    push(
        CaptionShelf::Label,
        "Label Blue",
        boxed(
            style(PATRICK, REGULAR, 52.0, WHITE),
            [30, 64, 175, 230],
            12.0,
            8.0,
        ),
    );
    push(
        CaptionShelf::Label,
        "Label Mint",
        boxed(
            style(PACIFICO, REGULAR, 48.0, [20, 40, 30, 255]),
            [167, 243, 208, 255],
            14.0,
            10.0,
        ),
    );
    push(
        CaptionShelf::Label,
        "Label Top",
        at(
            boxed(
                style(BE_VIETNAM, BOLD, 48.0, WHITE),
                [0, 0, 0, 160],
                12.0,
                6.0,
            ),
            CaptionPosition::Top,
        ),
    );

    // Karaoke: an outlined or boxed caption whose active word takes the highlight colour.
    push(
        CaptionShelf::Karaoke,
        "Karaoke Gold",
        highlight(
            outlined(style(BE_VIETNAM, BOLD, 48.0, WHITE), BLACK, 3.0),
            GOLD,
        ),
    );
    push(
        CaptionShelf::Karaoke,
        "Karaoke Cyan",
        highlight(
            outlined(style(BE_VIETNAM, BOLD, 48.0, WHITE), BLACK, 3.0),
            [0, 229, 255],
        ),
    );
    push(
        CaptionShelf::Karaoke,
        "Karaoke Pink",
        highlight(
            outlined(style(BE_VIETNAM, BOLD, 48.0, WHITE), BLACK, 3.0),
            [255, 64, 200],
        ),
    );
    push(
        CaptionShelf::Karaoke,
        "Karaoke Green",
        highlight(
            outlined(style(BE_VIETNAM, BOLD, 48.0, WHITE), BLACK, 3.0),
            [132, 255, 132],
        ),
    );
    push(
        CaptionShelf::Karaoke,
        "Karaoke Box",
        highlight(
            boxed(
                style(BE_VIETNAM, BOLD, 48.0, WHITE),
                [0, 0, 0, 160],
                12.0,
                6.0,
            ),
            GOLD,
        ),
    );
    push(
        CaptionShelf::Karaoke,
        "Karaoke Poster",
        highlight(
            outlined(style(ANTON, REGULAR, 60.0, WHITE), BLACK, 4.0),
            GOLD,
        ),
    );

    out
}

/// A caption style with the given family, face, size and colour; every other field at its
/// default: bottom position, 5 percent safe area, two lines, no outline, no box.
fn style(font: &str, font_style: &str, size: f32, color: [u8; 4]) -> CaptionStyle {
    CaptionStyle {
        font: font.into(),
        font_style: font_style.into(),
        size,
        color,
        outline: None,
        ..CaptionStyle::default()
    }
}

fn at(mut style: CaptionStyle, position: CaptionPosition) -> CaptionStyle {
    style.position = position;
    style
}

fn one_line(mut style: CaptionStyle) -> CaptionStyle {
    style.max_lines = 1;
    style
}

fn outlined(mut style: CaptionStyle, color: [u8; 4], width: f32) -> CaptionStyle {
    style.outline = Some(CaptionOutline { color, width });
    style
}

fn boxed(mut style: CaptionStyle, color: [u8; 4], padding: f32, radius: f32) -> CaptionStyle {
    style.background = Some(CaptionBackground {
        color,
        padding,
        radius,
    });
    style
}

fn highlight(mut style: CaptionStyle, color: [u8; 3]) -> CaptionStyle {
    style.word_highlight_color = color;
    style
}
