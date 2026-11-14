//! Starred catalogue items per catalogue family: app preference, never project data.

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
/// Starred items across every family; keeps the preference document bounded.
pub const MAX_CATALOG_FAVORITES: usize = 256;
/// Catalogue ids are short keys (effect kinds, sticker ids, animation words).
pub const MAX_CATALOG_FAVORITE_ID_BYTES: usize = 64;
/// Starred sounds, each with the metadata its Favorites row shows.
pub const MAX_SOUND_FAVORITES: usize = 32;
pub const MAX_SOUND_FAVORITE_TITLE_BYTES: usize = 128;
pub const MAX_SOUND_FAVORITE_URL_BYTES: usize = 256;

/// A starred sound's display metadata. Favorites lists it without a provider lookup.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundFavorite {
    /// The provider's sound id.
    pub id: u64,
    pub title: String,
    pub duration_ms: u64,
    /// The provider's preview URL.
    pub preview_url: String,
}

/// Starred items: ids of every family but `sounds`, plus starred sounds with their
/// metadata. Each list is ordered oldest star first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogFavorites {
    ids: BTreeMap<String, Vec<String>>,
    sounds: Vec<SoundFavorite>,
}

impl CatalogFavorites {
    pub fn contains(&self, family: &str, id: &str) -> bool {
        if family == "sounds" {
            return self.sounds.iter().any(|sound| sound.id.to_string() == id);
        }
        self.ids
            .get(family)
            .is_some_and(|ids| ids.iter().any(|known| known == id))
    }

    /// Starred ids of one id family, oldest star first. Sounds are read with `sounds`.
    pub fn list(&self, family: &str) -> &[String] {
        match self.ids.get(family) {
            Some(ids) => ids.as_slice(),
            None => &[],
        }
    }

    /// Starred sounds, oldest star first.
    pub fn sounds(&self) -> &[SoundFavorite] {
        &self.sounds
    }

    /// The document after starring or unstarring one id of a non-sound family. Refuses
    /// the `sounds` family, whose stars carry metadata (see `with_sound`), an unknown
    /// family, an empty or oversized id, and a document past the bound.
    pub fn with(&self, family: &str, id: &str, favorite: bool) -> Result<Self> {
        if family == "sounds" {
            return Err(Error::Invalid("catalog favourites"));
        }
        let mut next = self.clone();
        let ids = next.ids.entry(family.to_owned()).or_default();
        ids.retain(|known| known != id);
        if favorite {
            ids.push(id.to_owned());
        }
        if ids.is_empty() {
            next.ids.remove(family);
        }
        next.validate()?;
        Ok(next)
    }

    /// The document after starring or unstarring one sound. A star stores the sound's
    /// metadata; an unstar drops the entry with the same id.
    pub fn with_sound(&self, sound: SoundFavorite, favorite: bool) -> Result<Self> {
        let mut next = self.clone();
        next.sounds.retain(|known| known.id != sound.id);
        if favorite {
            next.sounds.push(sound);
        }
        next.validate()?;
        Ok(next)
    }

    pub(super) fn validate(&self) -> Result<()> {
        let total: usize = self.ids.values().map(Vec::len).sum::<usize>() + self.sounds.len();
        if total > MAX_CATALOG_FAVORITES || self.sounds.len() > MAX_SOUND_FAVORITES {
            return Err(Error::Invalid("catalog favourites"));
        }
        for (family, ids) in &self.ids {
            if family == "sounds"
                || !CATALOG_FAMILIES.contains(&family.as_str())
                || ids
                    .iter()
                    .any(|id| id.is_empty() || id.len() > MAX_CATALOG_FAVORITE_ID_BYTES)
                || ids.iter().collect::<BTreeSet<_>>().len() != ids.len()
            {
                return Err(Error::Invalid("catalog favourites"));
            }
        }
        for sound in &self.sounds {
            if sound.title.is_empty()
                || sound.title.len() > MAX_SOUND_FAVORITE_TITLE_BYTES
                || sound.preview_url.is_empty()
                || sound.preview_url.len() > MAX_SOUND_FAVORITE_URL_BYTES
            {
                return Err(Error::Invalid("catalog favourites"));
            }
        }
        let sound_ids: BTreeSet<u64> = self.sounds.iter().map(|sound| sound.id).collect();
        if sound_ids.len() != self.sounds.len() {
            return Err(Error::Invalid("catalog favourites"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sound(id: u64) -> SoundFavorite {
        SoundFavorite {
            id,
            title: format!("Sound {id}"),
            duration_ms: 1000,
            preview_url: format!("https://cdn.example/previews/{id}.mp3"),
        }
    }

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
    fn sound_stars_keep_metadata_in_order_and_unstar_by_id() {
        let starred = CatalogFavorites::default()
            .with_sound(sound(12), true)
            .and_then(|next| next.with_sound(sound(7), true))
            .and_then(|next| next.with_sound(sound(12), false))
            .expect("valid sound stars");
        assert_eq!(starred.sounds(), [sound(7)]);
        assert!(starred.contains("sounds", "7"));
        assert!(!starred.contains("sounds", "12"));
    }

    #[test]
    fn refuses_unknown_families_bad_ids_and_overflow() {
        let empty = CatalogFavorites::default();
        assert!(empty.with("unknown", "x", true).is_err());
        assert!(empty.with("sounds", "12", true).is_err());
        assert!(empty.with("effects", "", true).is_err());
        let long = "x".repeat(MAX_CATALOG_FAVORITE_ID_BYTES + 1);
        assert!(empty.with("effects", &long, true).is_err());
        let mut full = empty.clone();
        for index in 0..MAX_CATALOG_FAVORITES {
            full = full
                .with("effects", &index.to_string(), true)
                .expect("within bound");
        }
        assert!(full.with("effects", "overflow", true).is_err());
    }

    #[test]
    fn sound_favorites_are_bounded_and_refuse_bad_metadata() {
        let mut sounds = CatalogFavorites::default();
        for index in 0..MAX_SOUND_FAVORITES {
            sounds = sounds
                .with_sound(sound(index as u64), true)
                .expect("within sound bound");
        }
        assert!(sounds.with_sound(sound(999), true).is_err());
        let mut untitled = sound(1);
        untitled.title.clear();
        assert!(
            CatalogFavorites::default()
                .with_sound(untitled, true)
                .is_err()
        );
        let mut long_url = sound(2);
        long_url.preview_url = "x".repeat(MAX_SOUND_FAVORITE_URL_BYTES + 1);
        assert!(
            CatalogFavorites::default()
                .with_sound(long_url, true)
                .is_err()
        );
    }
}
