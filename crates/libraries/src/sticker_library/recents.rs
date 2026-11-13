//! The most recently inserted sticker ids, kept outside projects as a per-user preference.
use super::{FLAGS_PROVIDER, build_sticker_id, is_known_sticker_id, parse_sticker_id};
use std::io;
use std::path::Path;

/// Most recent sticker ids kept.
pub const MAX_RECENT_STICKERS: usize = 50;

/// Recent sticker ids, newest first, with no duplicates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recents {
    ids: Vec<String>,
}

impl Recents {
    /// Keeps known ids in order, dropping duplicates and anything past the cap.
    pub fn from_ids<I: IntoIterator<Item = String>>(ids: I) -> Self {
        let mut recents = Self::default();
        for id in ids {
            if recents.ids.len() >= MAX_RECENT_STICKERS {
                break;
            }
            if let Some(canonical) = canonical_id(&id)
                && !recents.ids.contains(&canonical)
            {
                recents.ids.push(canonical);
            }
        }
        recents
    }

    /// Recent ids, newest first.
    pub fn ids(&self) -> &[String] {
        &self.ids
    }

    /// Moves the id to the front, inserting it when absent. Unknown ids are refused.
    /// Returns whether the id was recorded.
    pub fn record(&mut self, sticker_id: &str) -> bool {
        let Some(canonical) = canonical_id(sticker_id) else {
            return false;
        };
        self.ids.retain(|existing| *existing != canonical);
        self.ids.insert(0, canonical);
        self.ids.truncate(MAX_RECENT_STICKERS);
        true
    }

    /// Removes every recent id.
    pub fn clear(&mut self) {
        self.ids.clear();
    }

    /// Reads the saved list. A missing file is an empty list; unreadable or malformed content is an error.
    pub fn load(path: &Path) -> io::Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        let ids: Vec<String> = serde_json::from_str(&text)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        Ok(Self::from_ids(ids))
    }

    /// Writes the list through a temporary file so a failed write leaves the previous list.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string(&self.ids)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, text)?;
        std::fs::rename(&temp, path)
    }
}

/// Trims and canonicalises a known id; `None` for anything the catalogs cannot resolve.
fn canonical_id(sticker_id: &str) -> Option<String> {
    if !is_known_sticker_id(sticker_id) {
        return None;
    }
    let (provider, value) = parse_sticker_id(sticker_id)?;
    let value = if provider == FLAGS_PROVIDER {
        value.to_uppercase()
    } else {
        value.to_owned()
    };
    Some(build_sticker_id(provider, &value))
}
