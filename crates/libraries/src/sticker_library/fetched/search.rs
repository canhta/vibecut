//! Search across the sticker categories: flags from the bundled set, emoji and icons from the
//! collections that are cached.
use super::collections::{Category, Collection, Entry};
use super::store::{CollectionState, StickerStore};
use crate::sticker_library::{FLAG_SIZE, FLAGS_PROVIDER, build_sticker_id, search_flags};
use engine::control::Result;

/// Intrinsic size recorded for every fetched sticker: their view boxes are square, and the
/// renderer rasterises at the destination size.
pub const FETCHED_SIZE: (u32, u32) = (256, 256);

/// One sticker a search returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// The `provider:value` id a project stores.
    pub id: String,
    pub name: String,
    pub category: Category,
    pub width: u32,
    pub height: u32,
}

/// A collection a search skipped because its files are not usable yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unavailable {
    pub collection: String,
    /// `NotDownloaded` or `Invalid`; the panel offers the download in both cases.
    pub state: CollectionState,
}

/// A page of results with the total before the limit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub items: Vec<Item>,
    pub total: usize,
    pub unavailable: Vec<Unavailable>,
}

impl StickerStore {
    /// Searches by name and keyword. `None` searches Emoji, Flags and Icons in that order; Shapes
    /// are graphic definitions and never appear here. An empty query lists everything. Every
    /// whitespace-separated word must occur in the name or a keyword; flags also match region and
    /// continent words. Emoji and icon rows come only from collections whose state is `Ready`;
    /// the others are listed in `unavailable` so the panel can offer the explicit download.
    ///
    /// # Errors
    ///
    /// IO failures while reading a collection's directory.
    pub fn search(&self, query: &str, category: Option<Category>, limit: usize) -> Result<Page> {
        let categories: &[Category] = match category {
            Some(Category::Emoji) => &[Category::Emoji],
            Some(Category::Flags) => &[Category::Flags],
            Some(Category::Shapes) => &[],
            Some(Category::Icons) => &[Category::Icons],
            None => &[Category::Emoji, Category::Flags, Category::Icons],
        };
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let mut items = Vec::new();
        let mut unavailable = Vec::new();
        for wanted in categories {
            if *wanted == Category::Flags {
                items.extend(
                    search_flags(query, usize::MAX)
                        .items
                        .iter()
                        .map(|flag| Item {
                            id: build_sticker_id(FLAGS_PROVIDER, flag.code),
                            name: flag.name.to_owned(),
                            category: Category::Flags,
                            width: FLAG_SIZE.0,
                            height: FLAG_SIZE.1,
                        }),
                );
                continue;
            }
            for collection in self.collections.iter().filter(|c| c.category == *wanted) {
                match self.state(&collection.id)? {
                    CollectionState::Ready => items.extend(
                        collection
                            .entries
                            .iter()
                            .filter(|entry| matches(entry, &words))
                            .map(|entry| item(collection, entry)),
                    ),
                    state => unavailable.push(Unavailable {
                        collection: collection.id.clone(),
                        state,
                    }),
                }
            }
        }
        let total = items.len();
        items.truncate(limit);
        Ok(Page {
            items,
            total,
            unavailable,
        })
    }

    /// The catalog row for a fetched `<collection>:<value>` id, whatever the cache holds. Flag
    /// ids resolve through [`crate::stickers::resolve_sticker`].
    pub fn resolve(&self, sticker_id: &str) -> Option<Item> {
        let (provider, value) = crate::sticker_library::parse_sticker_id(sticker_id)?;
        let collection = self.collection(provider)?;
        collection.entry(value).map(|entry| item(collection, entry))
    }
}

fn item(collection: &Collection, entry: &Entry) -> Item {
    Item {
        id: build_sticker_id(&collection.id, &entry.value),
        name: entry.name.clone(),
        category: collection.category,
        width: FETCHED_SIZE.0,
        height: FETCHED_SIZE.1,
    }
}

fn matches(entry: &Entry, words: &[String]) -> bool {
    let name = entry.name.to_lowercase();
    words.iter().all(|word| {
        name.contains(word.as_str())
            || entry
                .keywords
                .iter()
                .any(|keyword| keyword.contains(word.as_str()))
    })
}
