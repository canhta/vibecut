//! Presentation registries for the assets and properties panels.
//!
//! The reference source selects a panel view and a per-element property tab set
//! from small data tables. Keeping the order, labels, icons and persisted ids
//! here means tests and the preference store agree without a second list.

use crate::{catalog::Text, kit::Icon};
use serde::{Deserialize, Serialize};

/// The nine assets-panel tabs in the reference's vertical icon bar order.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum AssetsTab {
    #[default]
    Media,
    Sounds,
    Text,
    Stickers,
    Effects,
    Transitions,
    Captions,
    Adjustment,
    Settings,
}

impl AssetsTab {
    pub const ALL: [Self; 9] = [
        Self::Media,
        Self::Sounds,
        Self::Text,
        Self::Stickers,
        Self::Effects,
        Self::Transitions,
        Self::Captions,
        Self::Adjustment,
        Self::Settings,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Media => "media",
            Self::Sounds => "sounds",
            Self::Text => "text",
            Self::Stickers => "stickers",
            Self::Effects => "effects",
            Self::Transitions => "transitions",
            Self::Captions => "captions",
            Self::Adjustment => "adjustment",
            Self::Settings => "settings",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tab| tab.id() == id)
    }

    pub const fn label(self) -> Text {
        match self {
            Self::Media => Text::Media,
            Self::Sounds => Text::Sounds,
            Self::Text => Text::TextItem,
            Self::Stickers => Text::Stickers,
            Self::Effects => Text::Effects,
            Self::Transitions => Text::Transitions,
            Self::Captions => Text::Caption,
            Self::Adjustment => Text::Adjustment,
            Self::Settings => Text::Settings,
        }
    }

    pub(crate) const fn icon(self) -> Icon {
        match self {
            Self::Media => Icon::FolderOpen,
            Self::Sounds => Icon::MusicNotes,
            Self::Text => Icon::TextT,
            Self::Stickers => Icon::Sticker,
            Self::Effects => Icon::Sparkle,
            Self::Transitions => Icon::ArrowsLeftRight,
            Self::Captions => Icon::Subtitles,
            Self::Adjustment => Icon::SlidersHorizontal,
            Self::Settings => Icon::GearSix,
        }
    }
}

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
                PropertiesTab::Blending,
            ],
            Self::Video => &[
                PropertiesTab::Transform,
                PropertiesTab::Audio,
                PropertiesTab::Speed,
                PropertiesTab::Blending,
                PropertiesTab::Masks,
                PropertiesTab::Effects,
            ],
            Self::Image => &[
                PropertiesTab::Transform,
                PropertiesTab::Blending,
                PropertiesTab::Masks,
                PropertiesTab::Effects,
            ],
            Self::Sticker => &[
                PropertiesTab::Transform,
                PropertiesTab::Blending,
                PropertiesTab::Effects,
            ],
            Self::Graphic => &[
                PropertiesTab::Graphic,
                PropertiesTab::Transform,
                PropertiesTab::Blending,
                PropertiesTab::Masks,
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
    Blending,
    Masks,
    Effects,
    Graphic,
}

impl PropertiesTab {
    pub const ALL: [Self; 8] = [
        Self::Text,
        Self::Transform,
        Self::Audio,
        Self::Speed,
        Self::Blending,
        Self::Masks,
        Self::Effects,
        Self::Graphic,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Transform => "transform",
            Self::Audio => "audio",
            Self::Speed => "speed",
            Self::Blending => "blending",
            Self::Masks => "masks",
            Self::Effects => "effects",
            Self::Graphic => "graphic",
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
            Self::Blending => Text::Blending,
            Self::Masks => Text::Masks,
            Self::Effects => Text::Effects,
            Self::Graphic => Text::Graphic,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_ids_are_unique_and_round_trip() {
        for tab in AssetsTab::ALL {
            assert_eq!(AssetsTab::from_id(tab.id()), Some(tab));
        }
        for kind in ElementKind::ALL {
            assert_eq!(ElementKind::from_id(kind.id()), Some(kind));
            assert!(kind.tabs().contains(&kind.default_tab()), "{kind:?}");
            for tab in kind.tabs() {
                assert_eq!(PropertiesTab::from_id(tab.id()), Some(*tab));
            }
        }
        for tab in PropertiesTab::ALL {
            assert_eq!(PropertiesTab::from_id(tab.id()), Some(tab));
        }
        assert_eq!(AssetsTab::from_id("unknown"), None);
        assert_eq!(ElementKind::from_id("unknown"), None);
        assert_eq!(PropertiesTab::from_id("unknown"), None);
    }
}
