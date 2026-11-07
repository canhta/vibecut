//! One app-owned text its locales and its locale formatting.
//!
//! [`Text`] is the stable key set. Both locales must fill every key, so a
//! missing Vietnamese row is a compile error; `cargo xtask i18n` also rejects
//! empty rows, duplicate keys and keys no frontend code uses.
mod format;
mod locale;
mod text;

pub use format::{
    Plural, blend_label, date, decimal, export_duration, export_range, format, format_count,
    number, parse_decimal, signed,
};
pub use locale::Locale;
pub use text::{Text, text};
