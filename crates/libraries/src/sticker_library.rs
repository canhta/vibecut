//! Bundled sticker ids and the catalogs the Stickers panel searches.
mod catalog;
mod flags;
mod recents;
mod search;

pub use catalog::{FLAG_SIZE, SHAPE_SIZE, insert_sticker, resolve_sticker, search_catalog};
pub use recents::{MAX_RECENT_STICKERS, Recents};
pub use search::{
    DEFAULT_SEARCH_LIMIT, FlagMatch, FlagPage, ShapeMatch, search_flags, search_shapes,
};

/// Provider id of the bundled country flags.
pub const FLAGS_PROVIDER: &str = "flags";
/// Provider id of the bundled shapes.
pub const SHAPES_PROVIDER: &str = "shapes";
/// Shape values a shapes provider id may carry, in browse order.
pub const SHAPE_VALUES: &[&str] = &["square", "circle", "triangle", "hexagon", "diamond", "star"];

/// Splits `provider:value`, trimming both sides. `None` when the separator is missing, leading or trailing.
pub fn parse_sticker_id(sticker_id: &str) -> Option<(&str, &str)> {
    let trimmed = sticker_id.trim();
    let (provider, value) = trimmed.split_once(':')?;
    let (provider, value) = (provider.trim(), value.trim());
    (!provider.is_empty() && !value.is_empty()).then_some((provider, value))
}

/// Builds `provider:value`.
pub fn build_sticker_id(provider: &str, value: &str) -> String {
    format!("{provider}:{value}")
}

/// Whether the id names a known provider and a value that provider can resolve.
pub fn is_known_sticker_id(sticker_id: &str) -> bool {
    match parse_sticker_id(sticker_id) {
        Some((FLAGS_PROVIDER, value)) => flags::COUNTRIES
            .iter()
            .any(|country| country.code.eq_ignore_ascii_case(value)),
        Some((SHAPES_PROVIDER, value)) => SHAPE_VALUES.contains(&value),
        _ => false,
    }
}
