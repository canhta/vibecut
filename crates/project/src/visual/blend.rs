//! Persisted visual item blend selection; render owns execution.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum BlendMode {
    #[default]
    Normal,
    Darken,
    Multiply,
    ColorBurn,
    Lighten,
    Screen,
    PlusLighter,
    ColorDodge,
    Overlay,
    SoftLight,
    HardLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl BlendMode {
    pub const ALL: [Self; 17] = [
        Self::Normal,
        Self::Darken,
        Self::Multiply,
        Self::ColorBurn,
        Self::Lighten,
        Self::Screen,
        Self::PlusLighter,
        Self::ColorDodge,
        Self::Overlay,
        Self::SoftLight,
        Self::HardLight,
        Self::Difference,
        Self::Exclusion,
        Self::Hue,
        Self::Saturation,
        Self::Color,
        Self::Luminosity,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Darken => "darken",
            Self::Multiply => "multiply",
            Self::ColorBurn => "color-burn",
            Self::Lighten => "lighten",
            Self::Screen => "screen",
            Self::PlusLighter => "plus-lighter",
            Self::ColorDodge => "color-dodge",
            Self::Overlay => "overlay",
            Self::SoftLight => "soft-light",
            Self::HardLight => "hard-light",
            Self::Difference => "difference",
            Self::Exclusion => "exclusion",
            Self::Hue => "hue",
            Self::Saturation => "saturation",
            Self::Color => "color",
            Self::Luminosity => "luminosity",
        }
    }
}
