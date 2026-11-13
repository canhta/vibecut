//! Starred catalogue ids per catalogue family: app preference, never project data.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The catalogue families whose tiles can be starred.
pub const CATALOG_FAMILIES: [&str; 6] = [
    "effects",
    "transitions",
    "filters",
    "animations",
    "stickers",
    "sounds",
];
/// Starred ids across every family; keeps the preference document bounded.
pub const MAX_CATALOG_FAVORITES: usize = 256;
/// Catalogue ids are short keys (effect kinds, sticker ids, sound ids).
pub const MAX_CATALOG_FAVORITE_ID_BYTES: usize = 64;

/// Favourite ids keyed by catalogue family, in the order each was starred.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CatalogFavorites(BTreeMap<String, Vec<String>>);

impl CatalogFavorites {
    pub fn contains(&self, family: &str, id: &str) -> bool {
        self.0
            .get(family)
            .is_some_and(|ids| ids.iter().any(|known| known == id))
    }

    /// Starred ids of one family, oldest star first.
    pub fn list(&self, family: &str) -> &[String] {
        match self.0.get(family) {
            Some(ids) => ids.as_slice(),
            None => &[],
        }
    }

    /// The document after starring or unstarring one id. Refuses an unknown family,
    /// an empty or oversized id, and a document past the bound.
    pub fn with(&self, family: &str, id: &str, favorite: bool) -> Result<Self> {
        let mut next = self.clone();
        let ids = next.0.entry(family.to_owned()).or_default();
        ids.retain(|known| known != id);
        if favorite {
            ids.push(id.to_owned());
        }
        if ids.is_empty() {
            next.0.remove(family);
        }
        next.validate()?;
        Ok(next)
    }

    pub(super) fn validate(&self) -> Result<()> {
        let total: usize = self.0.values().map(Vec::len).sum();
        if total > MAX_CATALOG_FAVORITES {
            return Err(Error::Invalid("catalog favourites"));
        }
        for (family, ids) in &self.0 {
            if !CATALOG_FAMILIES.contains(&family.as_str())
                || ids
                    .iter()
                    .any(|id| id.is_empty() || id.len() > MAX_CATALOG_FAVORITE_ID_BYTES)
                || ids.iter().collect::<BTreeSet<_>>().len() != ids.len()
            {
                return Err(Error::Invalid("catalog favourites"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_and_unstar_keep_order_and_drop_empty_families() {
        let starred = CatalogFavorites::default()
            .with("effects", "blur", true)
            .and_then(|next| next.with("effects", "glow", true))
            .and_then(|next| next.with("effects", "blur", false))
            .expect("valid stars");
        assert_eq!(starred.list("effects"), ["glow"]);
        assert!(starred.contains("effects", "glow"));
        assert!(!starred.contains("effects", "blur"));
        let cleared = starred
            .with("effects", "glow", false)
            .expect("unstar is valid");
        assert_eq!(cleared, CatalogFavorites::default());
    }

    #[test]
    fn refuses_unknown_families_bad_ids_and_overflow() {
        let empty = CatalogFavorites::default();
        assert!(empty.with("unknown", "x", true).is_err());
        assert!(empty.with("effects", "", true).is_err());
        let long = "x".repeat(MAX_CATALOG_FAVORITE_ID_BYTES + 1);
        assert!(empty.with("effects", &long, true).is_err());
        let mut full = empty;
        for index in 0..MAX_CATALOG_FAVORITES {
            full = full
                .with("sounds", &index.to_string(), true)
                .expect("within bound");
        }
        assert!(full.with("sounds", "overflow", true).is_err());
    }
}
