//! The appearance copy every window reads: the UI locale and the dark theme.
//!
//! Widgets take a [`Look`] by value instead of borrowing the whole preference
//! document, so changing appearance stays a leaf decision at the call site.

use crate::i18n::Locale;

/// The locale and theme a window draws with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Look {
    pub locale: Locale,
    pub dark: bool,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            locale: Locale::En,
            dark: true,
        }
    }
}

impl Look {
    /// The catalog row for `key` in this look's locale.
    pub fn text(self, key: crate::i18n::Text) -> &'static str {
        crate::i18n::text(self.locale, key)
    }
}
