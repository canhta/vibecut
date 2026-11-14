//! The Text tab's library rows: the built-in styles (looks) and templates after the search,
//! the category chip and the All/Commercial filter. Both lists are bundled data, so the tab
//! works offline; the Commercial filter follows each entry's recorded licence.

use crate::i18n::{self, Locale, Text};
use project::{TextLook, TextShelf, TextTemplate};

/// The tab's two pages: the look tiles, or the template tiles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum TextPage {
    #[default]
    Styles,
    Templates,
}

impl TextPage {
    pub(super) const ALL: [Self; 2] = [Self::Styles, Self::Templates];

    pub(super) const fn id(self) -> &'static str {
        match self {
            Self::Styles => "styles",
            Self::Templates => "templates",
        }
    }

    pub(super) const fn label(self) -> Text {
        match self {
            Self::Styles => Text::TextLibraryStyles,
            Self::Templates => Text::TextLibraryTemplates,
        }
    }

    pub(super) const fn search_hint(self) -> Text {
        match self {
            Self::Styles => Text::TextLibrarySearchStyles,
            Self::Templates => Text::TextLibrarySearchTemplates,
        }
    }

    /// The catalogue-favourites family the page's stars belong to.
    pub(super) const fn family(self) -> &'static str {
        match self {
            Self::Styles => "text_looks",
            Self::Templates => "text_templates",
        }
    }
}

/// The category chip row: the star chip lists only favourites, a shelf chip one shelf. No chip
/// chosen lists every entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TextChip {
    Favorites,
    Shelf(TextShelf),
}

impl TextChip {
    /// Chips in row order: the star, then the shelves.
    pub(super) const ALL: [Self; 6] = [
        Self::Favorites,
        Self::Shelf(TextShelf::Plain),
        Self::Shelf(TextShelf::Outline),
        Self::Shelf(TextShelf::Shadow),
        Self::Shelf(TextShelf::Glow),
        Self::Shelf(TextShelf::Label),
    ];

    pub(super) const fn id(self) -> &'static str {
        match self {
            Self::Favorites => "favorites",
            Self::Shelf(TextShelf::Plain) => "plain",
            Self::Shelf(TextShelf::Outline) => "outline",
            Self::Shelf(TextShelf::Shadow) => "shadow",
            Self::Shelf(TextShelf::Glow) => "glow",
            Self::Shelf(TextShelf::Label) => "label",
        }
    }

    pub(super) const fn label(self) -> Text {
        match self {
            Self::Favorites => Text::AssetsFavorites,
            Self::Shelf(TextShelf::Plain) => Text::LookShelfPlain,
            Self::Shelf(TextShelf::Outline) => Text::LookShelfOutline,
            Self::Shelf(TextShelf::Shadow) => Text::LookShelfShadow,
            Self::Shelf(TextShelf::Glow) => Text::LookShelfGlow,
            Self::Shelf(TextShelf::Label) => Text::LookShelfLabel,
        }
    }
}

/// What a tile applies or inserts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Entry {
    Look(TextLook),
    Template(TextTemplate),
}

/// One tile: the entry, its wire id (the favourites key), its translated name and whether it is
/// starred.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Tile {
    pub(super) entry: Entry,
    pub(super) id: String,
    pub(super) name: &'static str,
    pub(super) starred: bool,
}

impl Tile {
    /// The look whose fill the tile previews.
    pub(super) const fn look(&self) -> TextLook {
        match self.entry {
            Entry::Look(look) => look,
            Entry::Template(template) => template.look(),
        }
    }
}

/// A case-insensitive substring test against the translated and the English name; an empty
/// needle matches everything.
fn matches(needle: &str, translated: &str, english: &str) -> bool {
    needle.is_empty()
        || translated.to_lowercase().contains(needle)
        || english.to_lowercase().contains(needle)
}

/// The tiles of one page after the search, the chip and the filter, in catalogue order.
/// `starred` is the page family's favourites list.
pub(super) fn tiles(
    page: TextPage,
    query: &str,
    chip: Option<TextChip>,
    commercial_only: bool,
    starred: &[String],
    locale: Locale,
) -> Vec<Tile> {
    let needle = query.trim().to_lowercase();
    let shelf = match chip {
        Some(TextChip::Shelf(shelf)) => Some(shelf),
        Some(TextChip::Favorites) | None => None,
    };
    let favorites_only = chip == Some(TextChip::Favorites);
    let mut found: Vec<(Tile, &'static str)> = Vec::new();
    match page {
        TextPage::Styles => {
            for look in project::search_text_looks("", shelf, commercial_only) {
                let id = look.id();
                found.push((
                    Tile {
                        entry: Entry::Look(look),
                        starred: starred.contains(&id),
                        id,
                        name: i18n::text(locale, i18n::text_look(look)),
                    },
                    look.name(),
                ));
            }
        }
        TextPage::Templates => {
            for template in project::search_text_templates("", shelf, commercial_only) {
                let id = template.id();
                found.push((
                    Tile {
                        entry: Entry::Template(template),
                        starred: starred.contains(&id),
                        id,
                        name: i18n::text(locale, i18n::text_template(template)),
                    },
                    template.name(),
                ));
            }
        }
    }
    found
        .into_iter()
        .filter(|(tile, english)| matches(&needle, tile.name, english))
        .map(|(tile, _)| tile)
        .filter(|tile| !favorites_only || tile.starred)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(tiles: &[Tile]) -> Vec<&str> {
        tiles.iter().map(|tile| tile.id.as_str()).collect()
    }

    #[test]
    fn an_empty_search_lists_every_entry_of_the_page() {
        let looks = tiles(TextPage::Styles, "", None, false, &[], Locale::En);
        assert_eq!(looks.len(), TextLook::ALL.len());
        let templates = tiles(TextPage::Templates, "", None, false, &[], Locale::En);
        assert_eq!(templates.len(), TextTemplate::ALL.len());
    }

    #[test]
    fn search_matches_the_translated_and_the_english_name() {
        let english = tiles(TextPage::Styles, " NEON ", None, false, &[], Locale::En);
        assert_eq!(ids(&english), ["neon_cyan", "neon_pink"]);
        let vietnamese = tiles(TextPage::Styles, "nhãn đen", None, false, &[], Locale::Vi);
        assert_eq!(ids(&vietnamese), ["label_black"]);
        // The English name still finds it under the Vietnamese catalog.
        let by_english = tiles(TextPage::Styles, "label black", None, false, &[], Locale::Vi);
        assert_eq!(ids(&by_english), ["label_black"]);
        assert!(tiles(TextPage::Styles, "zzz", None, false, &[], Locale::En).is_empty());
    }

    #[test]
    fn a_shelf_chip_keeps_one_shelf_and_the_star_chip_keeps_favourites() {
        let glow = tiles(
            TextPage::Styles,
            "",
            Some(TextChip::Shelf(TextShelf::Glow)),
            false,
            &[],
            Locale::En,
        );
        assert_eq!(glow.len(), 5);
        assert!(glow.iter().all(|tile| tile.look().shelf() == TextShelf::Glow));
        let starred = vec!["neon_pink".to_owned(), "clean_white".to_owned()];
        let favorites = tiles(
            TextPage::Styles,
            "",
            Some(TextChip::Favorites),
            false,
            &starred,
            Locale::En,
        );
        assert_eq!(ids(&favorites), ["clean_white", "neon_pink"]);
        assert!(favorites.iter().all(|tile| tile.starred));
        let none = tiles(
            TextPage::Styles,
            "",
            Some(TextChip::Favorites),
            false,
            &[],
            Locale::En,
        );
        assert!(none.is_empty());
    }

    #[test]
    fn the_commercial_filter_keeps_the_entries_with_a_commercial_licence() {
        let all = tiles(TextPage::Styles, "", None, false, &[], Locale::En);
        let commercial = tiles(TextPage::Styles, "", None, true, &[], Locale::En);
        assert_eq!(ids(&commercial), ids(&all));
        assert!(
            commercial
                .iter()
                .all(|tile| matches!(tile.entry, Entry::Look(look) if look.commercial()))
        );
        let templates = tiles(TextPage::Templates, "", None, true, &[], Locale::En);
        assert!(
            templates
                .iter()
                .all(|tile| matches!(tile.entry, Entry::Template(template) if template.commercial()))
        );
    }

    #[test]
    fn template_tiles_preview_their_looks_and_star_by_template_id() {
        let starred = vec!["neon_sign".to_owned()];
        let templates = tiles(TextPage::Templates, "", None, false, &starred, Locale::En);
        let neon = templates
            .iter()
            .find(|tile| tile.id == "neon_sign")
            .expect("template tile");
        assert!(neon.starred);
        assert_eq!(neon.look(), TextTemplate::NeonSign.look());
        assert_eq!(neon.name, "Neon Sign");
        assert!(
            templates
                .iter()
                .filter(|tile| tile.id != "neon_sign")
                .all(|tile| !tile.starred)
        );
    }
}
