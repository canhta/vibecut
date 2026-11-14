//! The font picker's persisted section: favourite families.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The font picker's persisted favourite family names.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontPickerPreferences {
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
