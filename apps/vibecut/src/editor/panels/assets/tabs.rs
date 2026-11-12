//! Presentation registry for the assets panel.
//!
//! The reference source selects a panel view from a small data table. Keeping the
//! order, labels, icons and persisted ids here means tests and the preference
//! store agree without a second list.

use crate::{i18n::Text, kit::Icon};
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::properties::{ElementKind, PropertiesTab};

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
