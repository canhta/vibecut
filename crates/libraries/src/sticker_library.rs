//! Bundled sticker ids and the flag catalog the Stickers panel searches. Stickers are flags only;
//! shapes are graphic items inserted by their own commands.
mod catalog;
mod flags;
mod recents;
mod search;

pub use catalog::{FLAG_SIZE, insert_sticker, resolve_sticker, search_catalog};
pub use recents::{MAX_RECENT_STICKERS, Recents};
pub use search::{DEFAULT_SEARCH_LIMIT, FlagMatch, FlagPage, search_flags};

/// Provider id of the bundled country flags.
pub const FLAGS_PROVIDER: &str = "flags";

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
