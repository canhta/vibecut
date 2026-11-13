//! Adapts the flag catalog to the control commands: browse and search pages, id resolution with
//! the intrinsic size, and the insert a panel or CLI applies to a session.
use super::{
    DEFAULT_SEARCH_LIMIT, FLAGS_PROVIDER, FlagMatch, build_sticker_id, parse_sticker_id,
    search_flags,
};
use engine::control::{StickerCategory, StickerEntry, StickerPage};
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

/// Browse and search page. Stickers are flags only, so every category lists the flags; the total
/// counts every match before `limit` (default [`DEFAULT_SEARCH_LIMIT`]) cuts the list.
pub fn search_catalog(query: &str, category: StickerCategory, limit: Option<u32>) -> StickerPage {
    let limit = limit.map_or(DEFAULT_SEARCH_LIMIT, |limit| limit as usize);
    let page = match category {
        StickerCategory::All | StickerCategory::Flags => search_flags(query, limit),
    };
    StickerPage {
        total: page.total as u64,
        results: page.items.iter().map(flag_entry).collect(),
    }
}

/// The catalog entry for a `flags:<code>` id, or `None` for any other id. Codes canonicalise to
/// the catalog's upper-case code.
pub fn resolve_sticker(sticker_id: &str) -> Option<StickerEntry> {
    let (provider, value) = parse_sticker_id(sticker_id)?;
    if provider != FLAGS_PROVIDER {
        return None;
    }
    search_flags("", usize::MAX)
        .items
        .iter()
        .find(|flag| flag.code.eq_ignore_ascii_case(value))
        .map(flag_entry)
}

/// Inserts a bundled sticker as one edit: the id is resolved here so the project stores the
/// canonical id and the intrinsic size of the asset it names.
///
/// # Errors
///
/// An unknown id, a duration below one tick, or any refusal of the insert itself.
pub fn insert_sticker(
    session: &mut Session,
    sticker_id: &str,
    placement: Placement,
    start: Tick,
    duration: Tick,
) -> Result<Outcome, EditError> {
    let entry = resolve_sticker(sticker_id).ok_or(EditError::Invalid("unknown sticker id"))?;
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
