//! Adapts the sticker catalog to the control commands: browse and search pages, id resolution with
//! the intrinsic size, and the insert a panel or CLI applies to a session.
use super::{
    Category, DEFAULT_SEARCH_LIMIT, FLAGS_PROVIDER, FlagMatch, Item, StickerStore,
    build_sticker_id, parse_sticker_id, search_flags,
};
use engine::control::{Result, StickerCategory, StickerEntry, StickerPage};
use engine::{Command, EditError, NewItem, Outcome, Placement, Session};
use time::{SourceRange, Tick};

/// Intrinsic size of every bundled flag, the SVG view box.
pub const FLAG_SIZE: (u32, u32) = (640, 480);

fn flag_entry(flag: &FlagMatch) -> StickerEntry {
    StickerEntry {
        id: build_sticker_id(FLAGS_PROVIDER, flag.code),
        name: flag.name.to_owned(),
        width: FLAG_SIZE.0,
        height: FLAG_SIZE.1,
    }
}

fn entry(item: &Item) -> StickerEntry {
    StickerEntry {
        id: item.id.clone(),
        name: item.name.clone(),
        width: item.width,
        height: item.height,
    }
}

/// Browse and search page over the bundled flags and, with a store, the Emoji and Icons
/// collections that are downloaded; `All` lists Emoji, Flags, Icons in that order. Without a
/// store only flags exist. The total counts every match before `limit` (default
/// [`DEFAULT_SEARCH_LIMIT`]) cuts the list.
///
/// # Errors
///
/// IO failures while reading a collection's directory.
pub fn search_catalog(
    store: Option<&StickerStore>,
    query: &str,
    category: StickerCategory,
    limit: Option<u32>,
) -> Result<StickerPage> {
    let limit = limit.map_or(DEFAULT_SEARCH_LIMIT, |limit| limit as usize);
    let wanted = match category {
        StickerCategory::All => None,
        StickerCategory::Emoji => Some(Category::Emoji),
        StickerCategory::Flags => Some(Category::Flags),
        StickerCategory::Icons => Some(Category::Icons),
    };
    if let Some(store) = store {
        let page = store.search(query, wanted, limit)?;
        return Ok(StickerPage {
            total: page.total as u64,
            results: page.items.iter().map(entry).collect(),
        });
    }
    if matches!(wanted, Some(Category::Emoji | Category::Icons)) {
        return Ok(StickerPage {
            total: 0,
            results: Vec::new(),
        });
    }
    let page = search_flags(query, limit);
    Ok(StickerPage {
        total: page.total as u64,
        results: page.items.iter().map(flag_entry).collect(),
    })
}

/// The catalog entry for a sticker id, or `None` for an id no collection owns. Flag codes
/// canonicalise to the catalog's upper-case code; a fetched id resolves whether or not its
/// collection is downloaded, and rendering refuses it until it is.
pub fn resolve_sticker(store: Option<&StickerStore>, sticker_id: &str) -> Option<StickerEntry> {
    let (provider, value) = parse_sticker_id(sticker_id)?;
    if provider == FLAGS_PROVIDER {
        return search_flags("", usize::MAX)
            .items
            .iter()
            .find(|flag| flag.code.eq_ignore_ascii_case(value))
            .map(flag_entry);
    }
    store?.resolve(sticker_id).as_ref().map(entry)
}

/// Inserts a sticker as one edit: the id is resolved here so the project stores the canonical id
/// and the intrinsic size of the asset it names.
///
/// # Errors
///
/// An unknown id, a duration below one tick, or any refusal of the insert itself.
pub fn insert_sticker(
    session: &mut Session,
    store: Option<&StickerStore>,
    sticker_id: &str,
    placement: Placement,
    start: Tick,
    duration: Tick,
) -> std::result::Result<Outcome, EditError> {
    let entry =
        resolve_sticker(store, sticker_id).ok_or(EditError::Invalid("unknown sticker id"))?;
    session.apply(Command::InsertItem {
        placement,
        item: NewItem {
            animations: Default::default(),
            source_asset: None,
            source_range: SourceRange::new(Tick::ZERO, duration).map_err(EditError::from)?,
            start,
            retime: Default::default(),
            content: project::ItemParams::Sticker(project::StickerParams::new(
                entry.id,
                entry.width,
                entry.height,
            )),
        },
    })
}
