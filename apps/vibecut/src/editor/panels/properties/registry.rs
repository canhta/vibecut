//! Maps a timeline element kind to its property tabs and back from persisted ids.
use crate::i18n::Text;
use serde::{Deserialize, Serialize};

/// A timeline element kind as presented in the properties panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    Text,
    Video,
    Image,
    Sticker,
    Graphic,
    Audio,
    Effect,
}

impl ElementKind {
    pub const ALL: [Self; 7] = [
        Self::Text,
        Self::Video,
        Self::Image,
        Self::Sticker,
        Self::Graphic,
        Self::Audio,
        Self::Effect,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Video => "video",
            Self::Image => "image",
            Self::Sticker => "sticker",
            Self::Graphic => "graphic",
            Self::Audio => "audio",
            Self::Effect => "effect",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }

    /// Type-specific tabs from the reference registry, in its order.
    pub const fn tabs(self) -> &'static [PropertiesTab] {
        match self {
            Self::Text => &[
                PropertiesTab::Text,
                PropertiesTab::Transform,
                PropertiesTab::Effects,
            ],
            // Basic sub-tabs in the reference order: Basic, Background, Remove BG, Mask.
            Self::Video => &[
                PropertiesTab::Transform,
                PropertiesTab::Background,
                PropertiesTab::RemoveBackground,
                PropertiesTab::Mask,
                PropertiesTab::Audio,
                PropertiesTab::Speed,
                PropertiesTab::Adjust,
                PropertiesTab::Effects,
            ],
            Self::Image => &[
                PropertiesTab::Transform,
                PropertiesTab::RemoveBackground,
                PropertiesTab::Mask,
                PropertiesTab::Adjust,
                PropertiesTab::Effects,
            ],
            Self::Sticker => &[PropertiesTab::Transform, PropertiesTab::Effects],
            Self::Graphic => &[
                PropertiesTab::Graphic,
                PropertiesTab::Transform,
                PropertiesTab::Effects,
            ],
            Self::Audio => &[PropertiesTab::Audio, PropertiesTab::Speed],
            Self::Effect => &[PropertiesTab::Effects],
        }
    }

    pub const fn default_tab(self) -> PropertiesTab {
        match self {
            Self::Text => PropertiesTab::Text,
            Self::Video | Self::Image | Self::Sticker => PropertiesTab::Transform,
            Self::Graphic => PropertiesTab::Graphic,
            Self::Audio => PropertiesTab::Audio,
            Self::Effect => PropertiesTab::Effects,
        }
    }
}

/// Tabs shared across element kinds. Persisted ids stay locale-independent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropertiesTab {
    Text,
    Transform,
    Audio,
    Speed,
    Mask,
    Adjust,
    RemoveBackground,
    Effects,
    Graphic,
    Background,
}

impl PropertiesTab {
    pub const ALL: [Self; 10] = [
        Self::Text,
        Self::Transform,
        Self::Audio,
        Self::Speed,
        Self::Mask,
        Self::Adjust,
        Self::RemoveBackground,
        Self::Effects,
        Self::Graphic,
        Self::Background,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Transform => "transform",
            Self::Audio => "audio",
            Self::Speed => "speed",
            Self::Mask => "mask",
            Self::Adjust => "adjust",
            Self::RemoveBackground => "remove_background",
            Self::Effects => "effects",
            Self::Graphic => "graphic",
            Self::Background => "background",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tab| tab.id() == id)
    }

    pub const fn label(self) -> Text {
        match self {
            Self::Text => Text::TextItem,
            Self::Transform => Text::Transform,
            Self::Audio => Text::Audio,
            Self::Speed => Text::Speed,
            Self::Mask => Text::Mask,
            Self::Adjust => Text::Adjust,
            Self::RemoveBackground => Text::RemoveBackground,
            Self::Effects => Text::Effects,
            Self::Graphic => Text::Graphic,
            Self::Background => Text::TextBackground,
        }
    }
}
