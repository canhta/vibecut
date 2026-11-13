//! Adapts the flag and shape catalogs to the control commands: browse and search pages, id
//! resolution with the intrinsic size, and the insert a panel or CLI applies to a session.
use super::{
    DEFAULT_SEARCH_LIMIT, FLAGS_PROVIDER, SHAPES_PROVIDER, build_sticker_id, parse_sticker_id,
    search_flags, search_shapes,
};
use engine::control::{StickerCategory, StickerEntry, StickerPage};
use engine::{Command, EditError, NewItem, Outcome, Placement, Session};
use time::{SourceRange, Tick};

/// Intrinsic size of every bundled flag, the SVG view box.
pub const FLAG_SIZE: (u32, u32) = (640, 480);
/// Intrinsic size of every shape: the 512-pixel author square of the graphic definitions.
pub const SHAPE_SIZE: (u32, u32) = (512, 512);

fn shape_name(value: &str) -> String {
    let mut chars = value.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn shape_entry(value: &str) -> StickerEntry {
    StickerEntry {
        id: build_sticker_id(SHAPES_PROVIDER, value),
        name: shape_name(value),
        width: SHAPE_SIZE.0,
        height: SHAPE_SIZE.1,
    }
}

/// Browse and search page for one category. `All` lists the shapes, then the flags; the total
/// counts every match before `limit` (default [`DEFAULT_SEARCH_LIMIT`]) cuts the list.
pub fn search_catalog(query: &str, category: StickerCategory, limit: Option<u32>) -> StickerPage {
    let limit = limit.map_or(DEFAULT_SEARCH_LIMIT, |limit| limit as usize);
    let shapes = match category {
        StickerCategory::All | StickerCategory::Shapes => search_shapes(query),
        StickerCategory::Flags => Vec::new(),
    };
    let flags = match category {
        StickerCategory::All | StickerCategory::Flags => Some(search_flags(query, limit)),
        StickerCategory::Shapes => None,
    };
    let total = shapes.len() + flags.as_ref().map_or(0, |page| page.total);
    let results = shapes
        .iter()
        .map(|shape| shape_entry(shape.value))
        .chain(
            flags
                .iter()
                .flat_map(|page| &page.items)
                .map(|flag| StickerEntry {
                    id: build_sticker_id(FLAGS_PROVIDER, flag.code),
                    name: flag.name.to_owned(),
                    width: FLAG_SIZE.0,
                    height: FLAG_SIZE.1,
                }),
        )
        .take(limit)
        .collect();
    StickerPage {
        total: total as u64,
        results,
    }
}

/// The catalog entry for a `provider:value` id, or `None` when the provider or value is unknown.
/// Flag values canonicalise to the catalog's upper-case code.
pub fn resolve_sticker(sticker_id: &str) -> Option<StickerEntry> {
    let (provider, value) = parse_sticker_id(sticker_id)?;
    match provider {
        FLAGS_PROVIDER => search_flags("", usize::MAX)
            .items
            .into_iter()
            .find(|flag| flag.code.eq_ignore_ascii_case(value))
            .map(|flag| StickerEntry {
                id: build_sticker_id(FLAGS_PROVIDER, flag.code),
                name: flag.name.to_owned(),
                width: FLAG_SIZE.0,
                height: FLAG_SIZE.1,
            }),
        SHAPES_PROVIDER => super::SHAPE_VALUES
            .contains(&value)
            .then(|| shape_entry(value)),
        _ => None,
    }
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
