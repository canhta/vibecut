//! The Stickers tab's rows and thumbnails. Shapes are the graphic cards of ticket 31, inserted
//! through their `graphic.insert.*` commands; stickers are the bundled flags. Recents hold both
//! kinds of id. Each thumbnail is one texture per id and pixel scale.

use crate::actions::GraphicShape;
use crate::i18n::{self, Locale, Text};
use crate::{Result, egui};
use engine::control::StickerCategory;
use libraries::stickers::{
    DEFAULT_SEARCH_LIMIT, FLAG_SIZE, FLAGS_PROVIDER, parse_sticker_id, resolve_sticker,
    search_catalog,
};
use std::{cell::RefCell, collections::BTreeMap};

/// Results shown before "See all" widens the page to the catalog's default limit.
const PREVIEW_RESULTS: usize = 24;
/// Thumbnail width in logical pixels; a flag keeps its 4:3 view box.
const THUMB_WIDTH: f32 = 64.0;

/// The Stickers tab's category tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum StickerTab {
    #[default]
    All,
    Flags,
    Shapes,
}

impl StickerTab {
    pub(super) const fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Flags => "flags",
            Self::Shapes => "shapes",
        }
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
    /// Matching shapes then flags, cut to the preview or the widened page.
    pub(super) results: Vec<Tile>,
    /// Every match before the cut.
    pub(super) total: usize,
    pub(super) expanded: bool,
}

/// Rows for one query, tab and expansion. `saved` is the persisted recent list.
pub(super) fn rows(
    query: &str,
    tab: StickerTab,
    expanded: bool,
    saved: &[String],
    locale: Locale,
) -> StickerRows {
    let limit = if expanded {
        DEFAULT_SEARCH_LIMIT
    } else {
        PREVIEW_RESULTS
    };
    let needle = query.trim().to_lowercase();
    let shapes: Vec<Tile> = if tab == StickerTab::Flags {
        Vec::new()
    } else {
        SHAPES
            .iter()
            .map(|(id, label, _)| Tile {
                id: (*id).to_owned(),
                name: i18n::text(locale, *label).to_owned(),
                shape: true,
            })
            .filter(|tile| needle.is_empty() || tile.name.to_lowercase().contains(&needle))
            .collect()
    };
    let flags = (tab != StickerTab::Shapes)
        .then(|| search_catalog(query, StickerCategory::Flags, Some(limit as u32)));
    let total = shapes.len() + flags.as_ref().map_or(0, |page| page.total as usize);
    let mut results = shapes;
    results.extend(
        flags
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
        known_recents(saved, locale)
            .into_iter()
            .filter(|tile| match tab {
                StickerTab::All => true,
                StickerTab::Flags => !tile.shape,
                StickerTab::Shapes => tile.shape,
            })
            .collect()
    } else {
        Vec::new()
    };
    StickerRows {
        recent,
        results,
        total,
        expanded,
    }
}

/// The saved ids that still resolve, as tiles, newest first.
fn known_recents(saved: &[String], locale: Locale) -> Vec<Tile> {
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
            resolve_sticker(id).map(|entry| Tile {
                id: entry.id,
                name: entry.name,
                shape: false,
            })
        })
        .collect()
}

/// The saved ids the Stickers tab can still insert, newest first. Stale ids drop here.
pub(super) fn known_ids(saved: &[String]) -> Vec<String> {
    known_recents(saved, Locale::En)
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
    if provider != FLAGS_PROVIDER {
        return Err(missing());
    }
    let width = side as u32;
    let height = (side * FLAG_SIZE.1 as f32 / FLAG_SIZE.0 as f32)
        .round()
        .max(1.0) as u32;
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
