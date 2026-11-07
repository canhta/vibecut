//! The offered UI languages and how an OS language tag maps to one.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Locale {
    #[default]
    En,
    Vi,
}
impl Locale {
    pub const fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Vi => "vi",
        }
    }
    /// The language's own name, for a language picker.
    pub const fn name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::Vi => "Tiếng Việt",
        }
    }
    /// Match a BCP 47 or POSIX tag (`vi`, `vi-VN`, `vi_VN.UTF-8`) to an
    /// offered locale. Anything not Vietnamese falls back to English.
    pub fn from_tag(tag: &str) -> Self {
        match tag
            .trim()
            .split(['-', '_', '.', '@'])
            .next()
            .unwrap_or_default()
        {
            language if language.eq_ignore_ascii_case("vi") => Self::Vi,
            _ => Self::En,
        }
    }
    /// The operating system's preferred language, English when unavailable.
    pub fn from_system() -> Self {
        sys_locale::get_locale()
            .as_deref()
            .map_or(Self::En, Self::from_tag)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn tags_map_to_an_offered_locale() {
        assert_eq!(Locale::En.code(), "en");
        assert_eq!(Locale::Vi.name(), "Tiếng Việt");
        for tag in ["vi", "vi-VN", "vi_VN.UTF-8", "VI-vn"] {
            assert_eq!(Locale::from_tag(tag), Locale::Vi, "{tag}");
        }
        for tag in ["en", "en-US", "fr-FR", "", "vibecut"] {
            assert_eq!(Locale::from_tag(tag), Locale::En, "{tag}");
        }
    }
}
