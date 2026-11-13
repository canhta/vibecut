//! The most recently inserted Stickers-panel ids, kept outside projects as a per-user preference.
//! The list holds opaque ids: flag sticker ids and the shape command ids. The app decides which
//! ids still resolve when it reads the list.

/// Most recent ids kept.
pub const MAX_RECENT_STICKERS: usize = 50;

/// Recent ids, newest first, with no duplicates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recents {
    ids: Vec<String>,
}

impl Recents {
    /// Keeps non-empty ids in order, dropping duplicates and anything past the cap.
    pub fn from_ids<I: IntoIterator<Item = String>>(ids: I) -> Self {
        let mut recents = Self::default();
        for id in ids {
            if recents.ids.len() >= MAX_RECENT_STICKERS {
                break;
            }
            let id = id.trim();
            if !id.is_empty() && !recents.ids.iter().any(|existing| existing == id) {
                recents.ids.push(id.to_owned());
            }
        }
        recents
    }

    /// Recent ids, newest first.
    pub fn ids(&self) -> &[String] {
        &self.ids
    }

    /// Moves the id to the front, inserting it when absent. Empty ids are refused.
    /// Returns whether the id was recorded.
    pub fn record(&mut self, id: &str) -> bool {
        let id = id.trim();
        if id.is_empty() {
            return false;
        }
        self.ids.retain(|existing| existing != id);
        self.ids.insert(0, id.to_owned());
        self.ids.truncate(MAX_RECENT_STICKERS);
        true
    }

    /// Removes every recent id.
    pub fn clear(&mut self) {
        self.ids.clear();
    }
}
