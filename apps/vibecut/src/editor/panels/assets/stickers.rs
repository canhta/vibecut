//! The Stickers tab's rows and thumbnails: the bundled catalog, the saved recents and one
//! texture per sticker id and pixel scale.

use crate::{Result, egui};
use engine::control::{StickerCategory, StickerEntry, StickerPage};
use libraries::stickers::{
    DEFAULT_SEARCH_LIMIT, FLAG_SIZE, FLAGS_PROVIDER, Recents, SHAPES_PROVIDER, parse_sticker_id,
    resolve_sticker, search_catalog,
};
use std::{cell::RefCell, collections::BTreeMap};

/// Results shown before "See all" widens the page to the catalog's default limit.
const PREVIEW_RESULTS: u32 = 24;
/// Thumbnail width in logical pixels; a flag keeps its 4:3 view box.
const THUMB_WIDTH: f32 = 64.0;

/// The Stickers tab's read-only rows, prepared before the frame paints.
pub(super) struct StickerRows {
    /// Saved ids that match the category, newest first. Empty while a search is typed.
    pub(super) recent: Vec<StickerEntry>,
    /// The first `PREVIEW_RESULTS`, or the widened page after See all.
    pub(super) page: StickerPage,
    pub(super) expanded: bool,
}

/// Rows for one query, category and expansion. `saved` is the persisted recent list.
pub(super) fn rows(
    query: &str,
    category: StickerCategory,
    expanded: bool,
    saved: &[String],
) -> StickerRows {
    let limit = if expanded {
        DEFAULT_SEARCH_LIMIT as u32
    } else {
        PREVIEW_RESULTS
    };
    let recent = if query.trim().is_empty() {
        Recents::from_ids(saved.iter().cloned())
            .ids()
            .iter()
            .filter_map(|id| resolve_sticker(id))
            .filter(|entry| in_category(entry, category))
            .collect()
    } else {
        Vec::new()
    };
    StickerRows {
        recent,
        page: search_catalog(query, category, Some(limit)),
        expanded,
    }
}

fn in_category(entry: &StickerEntry, category: StickerCategory) -> bool {
    let provider = parse_sticker_id(&entry.id).map(|(provider, _)| provider);
    match category {
        StickerCategory::All => true,
        StickerCategory::Flags => provider == Some(FLAGS_PROVIDER),
        StickerCategory::Shapes => provider == Some(SHAPES_PROVIDER),
    }
}

/// Thumbnail textures keyed by sticker id and pixel scale. A failed sticker keeps `None`, so a
/// frame never retries it.
#[derive(Default)]
pub(in crate::editor) struct Thumbnails {
    textures: RefCell<BTreeMap<(String, u32), Option<egui::TextureHandle>>>,
}

impl Thumbnails {
    pub(in crate::editor) fn texture(
        &self,
        ctx: &egui::Context,
        sticker_id: &str,
    ) -> Option<egui::TextureHandle> {
        let scale = ctx.pixels_per_point();
        let key = (sticker_id.to_owned(), (scale * 100.0).round() as u32);
        if let Some(cached) = self.textures.borrow().get(&key) {
            return cached.clone();
        }
        let texture = match thumbnail(sticker_id, scale) {
            Ok(image) => Some(ctx.load_texture(
                format!("sticker.{sticker_id}"),
                image,
                egui::TextureOptions::LINEAR,
            )),
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

/// Flags come from the render rasteriser. Shapes use the production compositor on the same
/// graphic definition a shape sticker draws on the timeline.
fn thumbnail(
    sticker_id: &str,
    scale: f32,
) -> std::result::Result<egui::ColorImage, render::StickerError> {
    let missing = || render::StickerError::Missing {
        sticker: sticker_id.to_owned(),
    };
    let (provider, value) = parse_sticker_id(sticker_id).ok_or_else(missing)?;
    let side = (THUMB_WIDTH * scale).round().max(1.0);
    if provider == SHAPES_PROVIDER {
        let graphic = render::sticker_shape_graphic(value).ok_or_else(missing)?;
        return graphic_image(graphic, side as u32).map_err(|_| missing());
    }
    let width = side as u32;
    let height = (side * FLAG_SIZE.1 as f32 / FLAG_SIZE.0 as f32)
        .round()
        .max(1.0) as u32;
    let raster = render::rasterize_sticker(sticker_id, width, height)?;
    Ok(egui::ColorImage::from_rgba_premultiplied(
        [width as usize, height as usize],
        &raster.pixels,
    ))
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
