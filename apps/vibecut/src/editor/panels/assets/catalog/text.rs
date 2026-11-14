//! The Text tab's library rows: the built-in styles (looks) and templates after the search and
//! the All/Commercial filter, grouped into the Favorites section and one section per shelf. Both
//! lists are bundled data, so the tab works offline; the Commercial filter follows each entry's
//! recorded licence. The tile backing keeps every sample readable.

use crate::i18n::{self, Locale, Text};
use project::{TextLook, TextShelf, TextTemplate};

/// The tab's two pages: the look tiles, or the template tiles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in super::super) enum TextPage {
    #[default]
    Styles,
    Templates,
}

impl TextPage {
    pub(in super::super) const ALL: [Self; 2] = [Self::Styles, Self::Templates];

    pub(in super::super) const fn id(self) -> &'static str {
        match self {
            Self::Styles => "styles",
            Self::Templates => "templates",
        }
    }

    pub(in super::super) const fn label(self) -> Text {
        match self {
            Self::Styles => Text::TextLibraryStyles,
            Self::Templates => Text::TextLibraryTemplates,
        }
    }

    pub(in super::super) const fn search_hint(self) -> Text {
        match self {
            Self::Styles => Text::TextLibrarySearchStyles,
            Self::Templates => Text::TextLibrarySearchTemplates,
        }
    }

    /// The catalogue-favourites family the page's stars belong to.
    pub(in super::super) const fn family(self) -> &'static str {
        match self {
            Self::Styles => "text_looks",
            Self::Templates => "text_templates",
        }
    }
}

/// One section of the list, and the chip that jumps to it: the Favorites section first, then
/// the five look shelves in panel order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) enum TextChip {
    Favorites,
    Shelf(TextShelf),
}

impl TextChip {
    /// Sections and chips in order.
    pub(in super::super) const ALL: [Self; 6] = [
        Self::Favorites,
        Self::Shelf(TextShelf::Plain),
        Self::Shelf(TextShelf::Outline),
        Self::Shelf(TextShelf::Shadow),
        Self::Shelf(TextShelf::Glow),
        Self::Shelf(TextShelf::Label),
    ];

    pub(in super::super) const fn id(self) -> &'static str {
        match self {
            Self::Favorites => "favorites",
            Self::Shelf(TextShelf::Plain) => "plain",
            Self::Shelf(TextShelf::Outline) => "outline",
            Self::Shelf(TextShelf::Shadow) => "shadow",
            Self::Shelf(TextShelf::Glow) => "glow",
            Self::Shelf(TextShelf::Label) => "label",
        }
    }

    pub(in super::super) const fn label(self) -> Text {
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
pub(in super::super) enum Entry {
    Look(TextLook),
    Template(TextTemplate),
}

/// One tile: the entry, its wire id (the favourites key), its translated name and whether it is
/// starred.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) struct Tile {
    pub(in super::super) entry: Entry,
    pub(in super::super) id: String,
    pub(in super::super) name: &'static str,
    pub(in super::super) starred: bool,
}

impl Tile {
    /// The look whose fill the tile previews.
    pub(in super::super) const fn look(&self) -> TextLook {
        match self.entry {
            Entry::Look(look) => look,
            Entry::Template(template) => template.look(),
        }
    }

    /// The shelf the tile's section belongs to.
    pub(in super::super) const fn shelf(&self) -> TextShelf {
        self.look().shelf()
    }
}

/// The tiles of one section: the starred ones for Favorites, else those of the shelf.
pub(in super::super) fn section(tiles: &[Tile], chip: TextChip) -> Vec<&Tile> {
    tiles
        .iter()
        .filter(|tile| match chip {
            TextChip::Favorites => tile.starred,
            TextChip::Shelf(shelf) => tile.shelf() == shelf,
        })
        .collect()
}

/// A case-insensitive substring test against the translated and the English name; an empty
/// needle matches everything.
fn matches(needle: &str, translated: &str, english: &str) -> bool {
    needle.is_empty()
        || translated.to_lowercase().contains(needle)
        || english.to_lowercase().contains(needle)
}

/// The tiles of one page after the search and the filter, in catalogue order. `starred` is the
/// page family's favourites list.
pub(in super::super) fn tiles(
    page: TextPage,
    query: &str,
    commercial_only: bool,
    starred: &[String],
    locale: Locale,
) -> Vec<Tile> {
    let needle = query.trim().to_lowercase();
    let mut found: Vec<(Tile, &'static str)> = Vec::new();
    match page {
        TextPage::Styles => {
            for look in project::search_text_looks("", None, commercial_only) {
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
            for template in project::search_text_templates("", None, commercial_only) {
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
        .collect()
}

/// The tile backing of the reference's Text grid: the neutral grey sampled from captures 103727
/// and 104218 (native sRGB 59, the same in both frames).
pub(in super::super) const BACKING: [u8; 3] = [59, 59, 59];
/// The backing a sample that is too dark for [`BACKING`] sits on: the mirrored grey (255 - 59).
pub(in super::super) const BACKING_LIGHT: [u8; 3] = [196, 196, 196];
/// The least contrast a tile sample keeps against its backing: WCAG 3:1, the bar for large
/// text (the sample is a 12 px semibold glyph run at 100 % zoom, so this is the floor, not the
/// goal).
pub(in super::super) const MIN_CONTRAST: f32 = 3.0;

/// WCAG relative luminance of an sRGB colour.
fn luminance(color: [u8; 3]) -> f32 {
    let linear = |channel: u8| {
        let value = f32::from(channel) / 255.0;
        if value <= 0.039_28 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color[0]) + 0.7152 * linear(color[1]) + 0.0722 * linear(color[2])
}

/// WCAG contrast ratio of two sRGB colours, 1 to 21.
pub(in super::super) fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The backing a sample in `ink` is drawn on: the reference grey when the sample reads on it,
/// else its light mirror.
pub(in super::super) fn backing(ink: [u8; 3]) -> [u8; 3] {
    if contrast(ink, BACKING) >= MIN_CONTRAST {
        BACKING
    } else {
        BACKING_LIGHT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(tiles: &[Tile]) -> Vec<&str> {
        tiles.iter().map(|tile| tile.id.as_str()).collect()
    }

    #[test]
    fn an_empty_search_lists_every_entry_of_the_page() {
        let looks = tiles(TextPage::Styles, "", false, &[], Locale::En);
        assert_eq!(looks.len(), TextLook::ALL.len());
        let templates = tiles(TextPage::Templates, "", false, &[], Locale::En);
        assert_eq!(templates.len(), TextTemplate::ALL.len());
    }

    #[test]
    fn search_matches_the_translated_and_the_english_name() {
        let english = tiles(TextPage::Styles, " NEON ", false, &[], Locale::En);
        assert_eq!(ids(&english), ["neon_cyan", "neon_pink"]);
        let vietnamese = tiles(TextPage::Styles, "nhãn đen", false, &[], Locale::Vi);
        assert_eq!(ids(&vietnamese), ["label_black"]);
        // The English name still finds it under the Vietnamese catalog.
        let by_english = tiles(TextPage::Styles, "label black", false, &[], Locale::Vi);
        assert_eq!(ids(&by_english), ["label_black"]);
        assert!(tiles(TextPage::Styles, "zzz", false, &[], Locale::En).is_empty());
    }

    #[test]
    fn sections_group_by_shelf_in_order_and_favorites_hold_the_starred() {
        let starred = vec!["neon_pink".to_owned(), "clean_white".to_owned()];
        let all = tiles(TextPage::Styles, "", false, &starred, Locale::En);
        let glow = section(&all, TextChip::Shelf(TextShelf::Glow));
        assert_eq!(glow.len(), 5);
        assert!(glow.iter().all(|tile| tile.shelf() == TextShelf::Glow));
        let favorites = section(&all, TextChip::Favorites);
        assert_eq!(
            favorites
                .iter()
                .map(|tile| tile.id.as_str())
                .collect::<Vec<_>>(),
            ["clean_white", "neon_pink"]
        );
        let total: usize = TextChip::ALL[1..]
            .iter()
            .map(|chip| section(&all, *chip).len())
            .sum();
        assert_eq!(
            total,
            all.len(),
            "every tile is in exactly one shelf section"
        );
        assert!(
            section(
                &tiles(TextPage::Styles, "", false, &[], Locale::En),
                TextChip::Favorites
            )
            .is_empty()
        );
    }

    #[test]
    fn the_commercial_filter_keeps_the_entries_with_a_commercial_licence() {
        let all = tiles(TextPage::Styles, "", false, &[], Locale::En);
        let commercial = tiles(TextPage::Styles, "", true, &[], Locale::En);
        assert_eq!(ids(&commercial), ids(&all));
        assert!(
            commercial
                .iter()
                .all(|tile| matches!(tile.entry, Entry::Look(look) if look.commercial()))
        );
        let templates = tiles(TextPage::Templates, "", true, &[], Locale::En);
        assert!(
            templates.iter().all(
                |tile| matches!(tile.entry, Entry::Template(template) if template.commercial())
            )
        );
    }

    #[test]
    fn template_tiles_preview_their_looks_and_star_by_template_id() {
        let starred = vec!["neon_sign".to_owned()];
        let templates = tiles(TextPage::Templates, "", false, &starred, Locale::En);
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

    #[test]
    fn contrast_follows_the_wcag_ratio() {
        assert!((contrast([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 0.01);
        assert!((contrast([59, 59, 59], [59, 59, 59]) - 1.0).abs() < 0.001);
    }

    #[test]
    fn every_look_and_template_sample_reads_on_its_backing() {
        let looks = tiles(TextPage::Styles, "", false, &[], Locale::En);
        let templates = tiles(TextPage::Templates, "", false, &[], Locale::En);
        assert_eq!(looks.len() + templates.len(), 36);
        for tile in looks.iter().chain(&templates) {
            let [red, green, blue, alpha] = tile.look().style().color.0;
            assert_eq!(alpha, 255, "{} is opaque", tile.id);
            let ink = [red, green, blue];
            let ratio = contrast(ink, backing(ink));
            assert!(
                ratio >= MIN_CONTRAST,
                "{} has {ratio:.2}:1 against its backing",
                tile.id
            );
        }
        // White samples keep the reference grey; Ink Black moves to the light mirror.
        assert_eq!(backing([255, 255, 255]), BACKING);
        let [red, green, blue, _] = TextLook::InkBlack.style().color.0;
        assert_eq!(backing([red, green, blue]), BACKING_LIGHT);
    }
}
