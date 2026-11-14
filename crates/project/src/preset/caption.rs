//! Built-in caption looks: thirty complete caption styles over the curated OFL families.
use captions::{CaptionBackground, CaptionOutline, CaptionPosition, CaptionStyle};

/// The caption look shelves in panel order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptionShelf {
    Plain,
    Outline,
    Label,
    Karaoke,
}

/// A built-in caption look, by stable snake_case wire id. Applying one sets the whole caption
/// style of an item; karaoke looks carry the word-highlight colour.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CaptionLook {
    PlainBottom,
    PlainMiddle,
    PlainTop,
    PlainBold,
    PlainLarge,
    PlainInk,
    PlainCream,
    PlainPoster,
    OutlineBottom,
    OutlineThin,
    OutlineThick,
    OutlineYellow,
    OutlineTop,
    OutlineMiddle,
    OutlinePoster,
    OutlineHand,
    LabelBlack,
    LabelWhite,
    LabelYellow,
    LabelPill,
    LabelRed,
    LabelBlue,
    LabelMint,
    LabelTop,
    KaraokeGold,
    KaraokeCyan,
    KaraokePink,
    KaraokeGreen,
    KaraokeBox,
    KaraokePoster,
}

impl CaptionLook {
    /// Every look, shelves in panel order and entries in shelf order.
    pub const ALL: [Self; 30] = [
        Self::PlainBottom,
        Self::PlainMiddle,
        Self::PlainTop,
        Self::PlainBold,
        Self::PlainLarge,
        Self::PlainInk,
        Self::PlainCream,
        Self::PlainPoster,
        Self::OutlineBottom,
        Self::OutlineThin,
        Self::OutlineThick,
        Self::OutlineYellow,
        Self::OutlineTop,
        Self::OutlineMiddle,
        Self::OutlinePoster,
        Self::OutlineHand,
        Self::LabelBlack,
        Self::LabelWhite,
        Self::LabelYellow,
        Self::LabelPill,
        Self::LabelRed,
        Self::LabelBlue,
        Self::LabelMint,
        Self::LabelTop,
        Self::KaraokeGold,
        Self::KaraokeCyan,
        Self::KaraokePink,
        Self::KaraokeGreen,
        Self::KaraokeBox,
        Self::KaraokePoster,
    ];

    /// The shelf the panel groups this look under.
    pub const fn shelf(self) -> CaptionShelf {
        match self {
            Self::PlainBottom
            | Self::PlainMiddle
            | Self::PlainTop
            | Self::PlainBold
            | Self::PlainLarge
            | Self::PlainInk
            | Self::PlainCream
            | Self::PlainPoster => CaptionShelf::Plain,
            Self::OutlineBottom
            | Self::OutlineThin
            | Self::OutlineThick
            | Self::OutlineYellow
            | Self::OutlineTop
            | Self::OutlineMiddle
            | Self::OutlinePoster
            | Self::OutlineHand => CaptionShelf::Outline,
            Self::LabelBlack
            | Self::LabelWhite
            | Self::LabelYellow
            | Self::LabelPill
            | Self::LabelRed
            | Self::LabelBlue
            | Self::LabelMint
            | Self::LabelTop => CaptionShelf::Label,
            Self::KaraokeGold
            | Self::KaraokeCyan
            | Self::KaraokePink
            | Self::KaraokeGreen
            | Self::KaraokeBox
            | Self::KaraokePoster => CaptionShelf::Karaoke,
        }
    }

    /// Our own display name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::PlainBottom => "Plain Bottom",
            Self::PlainMiddle => "Plain Middle",
            Self::PlainTop => "Plain Top",
            Self::PlainBold => "Plain Bold",
            Self::PlainLarge => "Plain Large",
            Self::PlainInk => "Plain Ink",
            Self::PlainCream => "Plain Cream",
            Self::PlainPoster => "Plain Poster",
            Self::OutlineBottom => "Outline Bottom",
            Self::OutlineThin => "Outline Thin",
            Self::OutlineThick => "Outline Thick",
            Self::OutlineYellow => "Outline Yellow",
            Self::OutlineTop => "Outline Top",
            Self::OutlineMiddle => "Outline Middle",
            Self::OutlinePoster => "Outline Poster",
            Self::OutlineHand => "Outline Hand",
            Self::LabelBlack => "Label Black",
            Self::LabelWhite => "Label White",
            Self::LabelYellow => "Label Yellow",
            Self::LabelPill => "Label Pill",
            Self::LabelRed => "Label Red",
            Self::LabelBlue => "Label Blue",
            Self::LabelMint => "Label Mint",
            Self::LabelTop => "Label Top",
            Self::KaraokeGold => "Karaoke Gold",
            Self::KaraokeCyan => "Karaoke Cyan",
            Self::KaraokePink => "Karaoke Pink",
            Self::KaraokeGreen => "Karaoke Green",
            Self::KaraokeBox => "Karaoke Box",
            Self::KaraokePoster => "Karaoke Poster",
        }
    }

    /// The complete caption style this look sets.
    pub fn style(self) -> CaptionStyle {
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
        match self {
            Self::PlainBottom => style(BE_VIETNAM, REGULAR, 48.0, WHITE),
            Self::PlainMiddle => at(
                style(BE_VIETNAM, REGULAR, 48.0, WHITE),
                CaptionPosition::Middle,
            ),
            Self::PlainTop => at(
                style(BE_VIETNAM, REGULAR, 48.0, WHITE),
                CaptionPosition::Top,
            ),
            Self::PlainBold => style(BE_VIETNAM, BOLD, 52.0, WHITE),
            Self::PlainLarge => one_line(style(BE_VIETNAM, BOLD, 64.0, WHITE)),
            Self::PlainInk => style(BE_VIETNAM, REGULAR, 44.0, INK),
            Self::PlainCream => style(PACIFICO, REGULAR, 48.0, [255, 244, 214, 255]),
            Self::PlainPoster => style(ANTON, REGULAR, 56.0, WHITE),
            Self::OutlineBottom => outlined(style(BE_VIETNAM, BOLD, 48.0, WHITE), BLACK, 3.0),
            Self::OutlineThin => outlined(style(BE_VIETNAM, REGULAR, 44.0, WHITE), BLACK, 2.0),
            Self::OutlineThick => outlined(style(BE_VIETNAM, BOLD, 52.0, WHITE), BLACK, 5.0),
            Self::OutlineYellow => outlined(
                style(BE_VIETNAM, BOLD, 48.0, [255, 215, 0, 255]),
                BLACK,
                3.0,
            ),
            Self::OutlineTop => at(
                outlined(style(BE_VIETNAM, BOLD, 48.0, WHITE), BLACK, 3.0),
                CaptionPosition::Top,
            ),
            Self::OutlineMiddle => at(
                outlined(style(BE_VIETNAM, BOLD, 48.0, WHITE), BLACK, 3.0),
                CaptionPosition::Middle,
            ),
            Self::OutlinePoster => outlined(style(ANTON, REGULAR, 60.0, WHITE), BLACK, 4.0),
            Self::OutlineHand => {
                outlined(style(PATRICK, REGULAR, 52.0, WHITE), [20, 20, 60, 255], 3.0)
            }
            Self::LabelBlack => boxed(
                style(BE_VIETNAM, BOLD, 48.0, WHITE),
                [0, 0, 0, 160],
                12.0,
                6.0,
            ),
            Self::LabelWhite => boxed(
                style(BE_VIETNAM, BOLD, 48.0, INK),
                [255, 255, 255, 230],
                12.0,
                6.0,
            ),
            Self::LabelYellow => boxed(
                style(BE_VIETNAM, BOLD, 48.0, INK),
                [255, 215, 0, 255],
                12.0,
                6.0,
            ),
            Self::LabelPill => boxed(
                style(BE_VIETNAM, BOLD, 46.0, WHITE),
                [0, 0, 0, 180],
                18.0,
                24.0,
            ),
            Self::LabelRed => boxed(
                style(ANTON, REGULAR, 56.0, WHITE),
                [220, 38, 38, 255],
                14.0,
                4.0,
            ),
            Self::LabelBlue => boxed(
                style(PATRICK, REGULAR, 52.0, WHITE),
                [30, 64, 175, 230],
                12.0,
                8.0,
            ),
            Self::LabelMint => boxed(
                style(PACIFICO, REGULAR, 48.0, [20, 40, 30, 255]),
                [167, 243, 208, 255],
                14.0,
                10.0,
            ),
            Self::LabelTop => at(
                boxed(
                    style(BE_VIETNAM, BOLD, 48.0, WHITE),
                    [0, 0, 0, 160],
                    12.0,
                    6.0,
                ),
                CaptionPosition::Top,
            ),
            Self::KaraokeGold => highlight(
                outlined(style(BE_VIETNAM, BOLD, 48.0, WHITE), BLACK, 3.0),
                GOLD,
            ),
            Self::KaraokeCyan => highlight(
                outlined(style(BE_VIETNAM, BOLD, 48.0, WHITE), BLACK, 3.0),
                [0, 229, 255],
            ),
            Self::KaraokePink => highlight(
                outlined(style(BE_VIETNAM, BOLD, 48.0, WHITE), BLACK, 3.0),
                [255, 64, 200],
            ),
            Self::KaraokeGreen => highlight(
                outlined(style(BE_VIETNAM, BOLD, 48.0, WHITE), BLACK, 3.0),
                [132, 255, 132],
            ),
            Self::KaraokeBox => highlight(
                boxed(
                    style(BE_VIETNAM, BOLD, 48.0, WHITE),
                    [0, 0, 0, 160],
                    12.0,
                    6.0,
                ),
                GOLD,
            ),
            Self::KaraokePoster => highlight(
                outlined(style(ANTON, REGULAR, 60.0, WHITE), BLACK, 4.0),
                GOLD,
            ),
        }
    }
}

impl CaptionLook {
    /// Whether a cue style is exactly this look's cue value.
    pub fn matches(self, style: &captions::CueStyle) -> bool {
        *style == self.cue_style()
    }

    /// The cue value a look materializes into on one caption item, including its max lines and
    /// word-highlight colour; an absent outline or box is a transparent colour, not a border.
    pub fn cue_style(self) -> captions::CueStyle {
        use captions::{CueAlignment, HorizontalAlignment, VerticalAlignment};
        let style = self.style();
        let (vertical, numpad) = match style.position {
            CaptionPosition::Top => (VerticalAlignment::Top, 8),
            CaptionPosition::Middle => (VerticalAlignment::Middle, 5),
            CaptionPosition::Bottom => (VerticalAlignment::Bottom, 2),
        };
        let (border_style, outline_color, background_color) =
            match (&style.background, &style.outline) {
                (Some(background), _) => (3, [0; 4], background.color),
                (None, Some(outline)) => (1, outline.color, [0; 4]),
                (None, None) => (1, [0; 4], [0; 4]),
            };
        let margin = style.safe_area_percent / 100.0;
        captions::CueStyle {
            font_family: style.font.as_str().into(),
            font_size_ratio: style.size / 1080.0,
            primary_color: style.color,
            outline_color,
            background_color,
            border_style,
            alignment: CueAlignment {
                numpad,
                horizontal: HorizontalAlignment::Center,
                vertical,
            },
            margin_left_ratio: margin,
            margin_right_ratio: margin,
            margin_vertical_ratio: margin,
            bold: style.font_style == "Bold",
            italic: false,
            underline: false,
            strikeout: false,
            max_lines: u8::try_from(style.max_lines).ok(),
            word_highlight_color: Some(style.word_highlight_color),
        }
    }
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
