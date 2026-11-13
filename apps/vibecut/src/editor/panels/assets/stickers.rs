//! The Stickers tab's rows, download offers and thumbnails. Shapes are the graphic cards of ticket
//! 31, inserted through their `graphic.insert.*` commands; stickers are the catalogue's flags,
//! emoji and icons. Recents hold every kind of id. Each thumbnail is one texture per id and pixel
//! scale.

use crate::actions::GraphicShape;
use crate::i18n::{self, Locale, Text};
use crate::{Result, egui};
use engine::ByteBudget;
use engine::control::StickerCategory;
use engine::jobs::{CancelToken, JobId, JobRunner, RunnerConfig, SystemClock};
use libraries::stickers::{
    Category, CollectionState, DEFAULT_SEARCH_LIMIT, FLAG_SIZE, FLAGS_PROVIDER, StickerStore,
    native_store, parse_sticker_id, resolve_sticker, search_catalog,
};
use std::{cell::RefCell, collections::BTreeMap, sync::Arc, time::Duration};

/// Results shown before "See all" widens the page to the catalog's default limit.
const PREVIEW_RESULTS: usize = 24;
/// Thumbnail width in logical pixels; a flag keeps its 4:3 view box.
const THUMB_WIDTH: f32 = 64.0;

/// The Stickers tab's category tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum StickerTab {
    #[default]
    All,
    Emoji,
    Icons,
    Flags,
    Shapes,
}

impl StickerTab {
    pub(super) const fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Emoji => "emoji",
            Self::Icons => "icons",
            Self::Flags => "flags",
            Self::Shapes => "shapes",
        }
    }

    /// The catalogue filter of the tab. Shapes are graphic cards, not catalogue entries.
    fn category(self) -> Option<StickerCategory> {
        match self {
            Self::All => Some(StickerCategory::All),
            Self::Emoji => Some(StickerCategory::Emoji),
            Self::Icons => Some(StickerCategory::Icons),
            Self::Flags => Some(StickerCategory::Flags),
            Self::Shapes => None,
        }
    }

    /// The collection categories whose download the tab offers. `All` lists what is ready only.
    fn offer_categories(self) -> &'static [Category] {
        match self {
            Self::Emoji => &[Category::Emoji],
            Self::Icons => &[Category::Icons],
            Self::All | Self::Flags | Self::Shapes => &[],
        }
    }

    /// Whether a saved id belongs to the tab. Emoji and icon ids belong to the tab of their
    /// collection's category.
    fn holds(self, tile: &Tile, store: Option<&StickerStore>) -> bool {
        let provider = parse_sticker_id(&tile.id).map(|(provider, _)| provider);
        match self {
            Self::All => true,
            Self::Shapes => tile.shape,
            Self::Flags => !tile.shape && provider == Some(FLAGS_PROVIDER),
            Self::Emoji | Self::Icons => {
                let wanted = if self == Self::Emoji {
                    Category::Emoji
                } else {
                    Category::Icons
                };
                !tile.shape
                    && store.is_some_and(|store| {
                        store.collections().iter().any(|collection| {
                            Some(collection.id.as_str()) == provider
                                && collection.category == wanted
                        })
                    })
            }
        }
    }
}

/// A collection the tab offers to download: its name, the licence credit shown with the explicit
/// action, and the size of the download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Offer {
    pub(super) collection: String,
    pub(super) name: String,
    pub(super) author: String,
    pub(super) license: String,
    pub(super) source: String,
    pub(super) bytes: u64,
}

/// Why the last download of a collection ended without files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum DownloadProblem {
    /// The transfer could not reach the server; the card says so and offers a retry.
    Offline,
    /// Any other failure; the cache keeps nothing from it.
    Failed,
}

/// The collection download that runs now: its job, cancel handle, progress and whether cancel
/// was asked.
pub(in crate::editor) struct Active {
    pub(in crate::editor) collection: String,
    pub(in crate::editor) job: JobId,
    pub(in crate::editor) cancel: CancelToken,
    pub(in crate::editor) progress: f32,
    pub(in crate::editor) cancelling: bool,
}

/// Owner of the Emoji and Icons downloads: one runner, one collection at a time, and the last
/// problem until the next attempt on that collection.
pub(in crate::editor) struct StickerDownloads {
    pub(in crate::editor) runner: JobRunner,
    pub(in crate::editor) active: Option<Active>,
    pub(in crate::editor) problem: Option<(String, DownloadProblem)>,
}

impl Drop for StickerDownloads {
    fn drop(&mut self) {
        self.runner.shutdown();
    }
}

impl StickerDownloads {
    pub(in crate::editor) fn new(budget: ByteBudget) -> Result<Self> {
        let runner = JobRunner::new(
            RunnerConfig {
                concurrency: [1, 1, 1],
                queue: 1,
                retained: 4,
                network_attempts: 1,
                backoff: Duration::from_millis(250),
                progress_interval: Duration::from_millis(40),
            },
            budget,
            Arc::new(SystemClock::default()),
        )?;
        Ok(Self {
            runner,
            active: None,
            problem: None,
        })
    }
}

/// The Shapes cards: command id, label and the graphic each card inserts.
pub(super) const SHAPES: [(&str, Text, GraphicShape); 4] = [
    (
        "graphic.insert.rectangle",
        Text::GraphicRectangle,
        GraphicShape::Rectangle,
    ),
    (
        "graphic.insert.ellipse",
        Text::GraphicEllipse,
        GraphicShape::Ellipse,
    ),
    (
        "graphic.insert.polygon",
        Text::GraphicPolygon,
        GraphicShape::Polygon,
    ),
    ("graphic.insert.star", Text::GraphicStar, GraphicShape::Star),
];

/// One tile: the id it inserts (a command id for shapes, a `provider:value` id for flags) and its
/// localised or catalogue name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Tile {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) shape: bool,
}

/// The Stickers tab's read-only rows, prepared before the frame paints.
pub(super) struct StickerRows {
    /// Saved entries that match the tab, newest first. Empty while a search is typed.
    pub(super) recent: Vec<Tile>,
    /// Matching shapes then catalogue entries, cut to the preview or the widened page.
    pub(super) results: Vec<Tile>,
    /// Every match before the cut.
    pub(super) total: usize,
    pub(super) expanded: bool,
    /// Collections of the tab that are not downloaded or not verified, for the download card.
    pub(super) offers: Vec<Offer>,
}

/// Rows for one query, tab and expansion. `saved` is the persisted recent list and `store` the
/// catalogue the Emoji and Icons collections come from; `None` leaves only the flags.
pub(super) fn rows(
    query: &str,
    tab: StickerTab,
    expanded: bool,
    saved: &[String],
    locale: Locale,
    store: Option<&StickerStore>,
) -> StickerRows {
    let limit = if expanded {
        DEFAULT_SEARCH_LIMIT
    } else {
        PREVIEW_RESULTS
    };
    let needle = query.trim().to_lowercase();
    let shapes: Vec<Tile> = if matches!(tab, StickerTab::All | StickerTab::Shapes) {
        SHAPES
            .iter()
            .map(|(id, label, _)| Tile {
                id: (*id).to_owned(),
                name: i18n::text(locale, *label).to_owned(),
                shape: true,
            })
            .filter(|tile| needle.is_empty() || tile.name.to_lowercase().contains(&needle))
            .collect()
    } else {
        Vec::new()
    };
    // A store read failure shows no catalogue entries rather than a stale page.
    let catalogue = tab
        .category()
        .and_then(|category| search_catalog(store, query, category, Some(limit as u32)).ok());
    let total = shapes.len() + catalogue.as_ref().map_or(0, |page| page.total as usize);
    let mut results = shapes;
    results.extend(
        catalogue
            .into_iter()
            .flat_map(|page| page.results)
            .map(|entry| Tile {
                id: entry.id,
                name: entry.name,
                shape: false,
            }),
    );
    results.truncate(limit);
    let recent = if needle.is_empty() {
        known_recents(saved, locale, store)
            .into_iter()
            .filter(|tile| tab.holds(tile, store))
            .collect()
    } else {
        Vec::new()
    };
    StickerRows {
        recent,
        results,
        total,
        expanded,
        offers: store.map_or_else(Vec::new, |store| offers(store, tab)),
    }
}

/// The collections of the tab that have no verified files yet, in catalogue order.
fn offers(store: &StickerStore, tab: StickerTab) -> Vec<Offer> {
    let categories = tab.offer_categories();
    store
        .collections()
        .iter()
        .filter(|collection| categories.contains(&collection.category))
        .filter(|collection| {
            store
                .state(&collection.id)
                .is_ok_and(|state| state != CollectionState::Ready)
        })
        .map(|collection| Offer {
            collection: collection.id.clone(),
            name: collection.name.clone(),
            author: collection.author.clone(),
            license: collection.license.clone(),
            source: collection.source(),
            bytes: collection.download_bytes(),
        })
        .collect()
}

/// The saved ids that still resolve, as tiles, newest first.
fn known_recents(saved: &[String], locale: Locale, store: Option<&StickerStore>) -> Vec<Tile> {
    saved
        .iter()
        .filter_map(|id| {
            if let Some((_, label, _)) = SHAPES.iter().find(|(shape, _, _)| *shape == id.as_str()) {
                return Some(Tile {
                    id: id.clone(),
                    name: i18n::text(locale, *label).to_owned(),
                    shape: true,
                });
            }
            resolve_sticker(store, id).map(|entry| Tile {
                id: entry.id,
                name: entry.name,
                shape: false,
            })
        })
        .collect()
}

/// The saved ids the Stickers tab can still insert, newest first. Stale ids drop here.
pub(super) fn known_ids(saved: &[String]) -> Vec<String> {
    known_recents(saved, Locale::En, native_store())
        .into_iter()
        .map(|tile| tile.id)
        .collect()
}

/// Thumbnail textures keyed by id and pixel scale. A failed id keeps `None`, so a frame never
/// retries it.
#[derive(Default)]
pub(in crate::editor) struct Thumbnails {
    textures: RefCell<BTreeMap<(String, u32), Option<egui::TextureHandle>>>,
}

impl Thumbnails {
    pub(in crate::editor) fn texture(
        &self,
        ctx: &egui::Context,
        id: &str,
    ) -> Option<egui::TextureHandle> {
        let scale = ctx.pixels_per_point();
        let key = (id.to_owned(), (scale * 100.0).round() as u32);
        if let Some(cached) = self.textures.borrow().get(&key) {
            return cached.clone();
        }
        let texture = match thumbnail(id, scale) {
            Ok(image) => {
                Some(ctx.load_texture(format!("sticker.{id}"), image, egui::TextureOptions::LINEAR))
            }
            Err(_) => {
                tracing::warn!(
                    category = "sticker_thumbnail",
                    "sticker thumbnail unavailable"
                );
                None
            }
        };
        self.textures.borrow_mut().insert(key, texture.clone());
        texture
    }
}

/// Shapes use the production compositor on the graphic a card inserts. Flags come from the
/// render rasteriser.
fn thumbnail(id: &str, scale: f32) -> std::result::Result<egui::ColorImage, render::StickerError> {
    let missing = || render::StickerError::Missing {
        sticker: id.to_owned(),
    };
    let side = (THUMB_WIDTH * scale).round().max(1.0);
    if let Some((_, _, shape)) = SHAPES.iter().find(|(shape, _, _)| *shape == id) {
        return graphic_image(graphic_of(*shape), side as u32).map_err(|_| missing());
    }
    let (provider, _) = parse_sticker_id(id).ok_or_else(missing)?;
    // Flags keep their 4:3 view box; fetched emoji and icons are square.
    let width = side as u32;
    let height = if provider == FLAGS_PROVIDER {
        (side * FLAG_SIZE.1 as f32 / FLAG_SIZE.0 as f32)
            .round()
            .max(1.0) as u32
    } else {
        width
    };
    let raster = render::rasterize_sticker(id, width, height)?;
    Ok(egui::ColorImage::from_rgba_premultiplied(
        [width as usize, height as usize],
        &raster.pixels,
    ))
}

/// The graphic definition a shape card inserts.
fn graphic_of(shape: GraphicShape) -> project::GraphicParams {
    match shape {
        GraphicShape::Rectangle => project::GraphicParams::rectangle(),
        GraphicShape::Ellipse => project::GraphicParams::ellipse(),
        GraphicShape::Polygon => project::GraphicParams::polygon(),
        GraphicShape::Star => project::GraphicParams::star(),
    }
}

/// One shape drawn at `side` x `side` pixels through the production compositor.
fn graphic_image(graphic: project::GraphicParams, side: u32) -> Result<egui::ColorImage> {
    let mut data = project::Project::empty(project::ProjectId(0), "".into())?.into_data();
    data.canvas.width = 512;
    data.canvas.height = 512;
    data.canvas.background = project::Color([0; 4]);
    data.scene_mut().timeline.main.items.push(project::Item {
        hidden: false,
        blend_mode: Default::default(),
        masks: vec![],
        effects: vec![],
        animations: Default::default(),
        id: project::ItemId(0),
        source_asset: None,
        source_range: time::SourceRange::new(time::Tick::ZERO, time::Tick(1))?,
        start: time::Tick::ZERO,
        retime: Default::default(),
        content: project::ItemParams::Graphic(graphic),
    });
    let project = project::Project::new(data)?;
    let mut settings = render::RenderSettings::from_project(&project);
    settings.width = side;
    settings.height = side;
    let budget = engine::ByteBudget::new(4 << 20)?;
    let hooks = engine::ByteBudgetHooks::new(&budget, None);
    let image = render::Renderer::new(&project, settings)?
        .render_frame(
            time::Tick::ZERO,
            &mut render::FileSources::default(),
            &hooks,
        )?
        .rgba8(13, &hooks)?;
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [side as usize, side as usize],
        &image.bytes,
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "panel fixtures")]
mod tests {
    use super::*;

    /// A store over an empty cache: every curated collection is not downloaded.
    fn empty_store(root: &std::path::Path) -> StickerStore {
        StickerStore::new(
            root.to_path_buf(),
            libraries::stickers::curated_collections(),
            Arc::new(transport::UreqTransport::new()),
        )
        .unwrap()
    }

    #[test]
    fn emoji_and_icons_tabs_offer_the_undownloaded_collections_with_their_credit() {
        let root = tempfile::tempdir().unwrap();
        let store = empty_store(root.path());
        let ids = |tab| {
            rows("", tab, false, &[], Locale::En, Some(&store))
                .offers
                .into_iter()
                .map(|offer| offer.collection)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(StickerTab::Emoji), ["noto"]);
        assert_eq!(ids(StickerTab::Icons), ["lucide", "phosphor"]);
        assert!(ids(StickerTab::All).is_empty());
        assert!(ids(StickerTab::Flags).is_empty());

        let offer = rows("", StickerTab::Emoji, false, &[], Locale::En, Some(&store))
            .offers
            .remove(0);
        assert!(!offer.author.is_empty() && !offer.license.is_empty());
        assert!(offer.source.starts_with("https://"));
        assert!(offer.bytes > 0);

        // Nothing is cached, so the emoji tab lists no entries and the flags stay in All.
        let emoji = rows("", StickerTab::Emoji, false, &[], Locale::En, Some(&store));
        assert_eq!(emoji.total, 0);
        assert!(emoji.results.is_empty());
        let all = rows(
            "asia",
            StickerTab::All,
            false,
            &[],
            Locale::En,
            Some(&store),
        );
        assert!(all.results.iter().all(|tile| !tile.shape));
        assert!(all.total > 0);
    }

    #[test]
    fn tabs_hold_only_their_own_recent_ids() {
        let root = tempfile::tempdir().unwrap();
        let store = empty_store(root.path());
        let saved = ["graphic.insert.star".to_owned(), "flags:VN".to_owned()];
        let shapes = rows(
            "",
            StickerTab::Shapes,
            false,
            &saved,
            Locale::En,
            Some(&store),
        );
        assert_eq!(shapes.recent.len(), 1);
        let flags = rows(
            "",
            StickerTab::Flags,
            false,
            &saved,
            Locale::En,
            Some(&store),
        );
        assert_eq!(flags.recent.len(), 1);
        assert_eq!(flags.recent[0].id, "flags:VN");
        let emoji = rows(
            "",
            StickerTab::Emoji,
            false,
            &saved,
            Locale::En,
            Some(&store),
        );
        assert!(emoji.recent.is_empty());
    }
}
