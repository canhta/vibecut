//! The font picker's persisted section: source tab and favourite families.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The font picker's source tabs. `MyFonts` stays hidden until font import
/// exists; `Favorites` is app behaviour backed by the preference store.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontTab {
    #[default]
    All,
    MyFonts,
    Favorites,
}
impl FontTab {
    pub const fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::MyFonts => "my_fonts",
            Self::Favorites => "favorites",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        [Self::All, Self::MyFonts, Self::Favorites]
            .into_iter()
            .find(|tab| tab.id() == id)
    }
}
/// The font picker's persisted tab and favourite family names.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontPickerPreferences {
    #[serde(default)]
    pub tab: FontTab,
    #[serde(default)]
    pub favorites: Vec<String>,
}
const MAX_FONT_FAVORITES: usize = 256;
const MAX_FONT_FAMILY_BYTES: usize = 128;
impl FontPickerPreferences {
    pub(super) fn validate(&self) -> Result<()> {
        if self.favorites.len() > MAX_FONT_FAVORITES
            || self
                .favorites
                .iter()
                .any(|family| family.is_empty() || family.len() > MAX_FONT_FAMILY_BYTES)
            || self.favorites.iter().collect::<BTreeSet<_>>().len() != self.favorites.len()
        {
            return Err(Error::Invalid("font favourites"));
        }
        Ok(())
    }
}
