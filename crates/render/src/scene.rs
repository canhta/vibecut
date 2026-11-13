//! Evaluates project items into one RGBA frame.
pub mod blend;
mod caption;
pub mod color;
mod draw;
pub mod effects;
mod evaluation;
pub mod mask;
mod raster;
pub(crate) mod transform;
pub use transform::arrange;
pub mod transition_thumbnail;
pub mod transitions;
pub mod viewport;

use crate::{Error, Image, Result};
pub use caption::style_from_cue;
pub use captions::{CaptionBackground, CaptionOutline, CaptionPosition, CaptionStyle};
use project::{AnimationPath, ItemId, Project};
pub use raster::sticker::{
    FLAGS_PROVIDER, MAX_STICKER_RASTER_SIDE, StickerError, StickerRaster, rasterize_sticker,
};
use raster::{TextEntry, TextKey};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Default for Rect {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    }
}
impl Rect {
    /// Validate finite, nonempty placement geometry within the compositor's bounds.
    fn validate(self) -> Result<()> {
        if [self.x, self.y, self.width, self.height]
            .iter()
            .any(|v| !v.is_finite())
            || self.width <= 0.0
            || self.height <= 0.0
            || self.width > 16.0
            || self.height > 16.0
            || self.x.abs() > 16.0
            || self.y.abs() > 16.0
        {
            return Err(Error::Invalid("normalized rectangle"));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Fit {
    #[default]
    Cover,
    Contain,
    Stretch,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub destination: Rect,
    /// Source-normalized region shown before placement fitting; project owns the type.
    pub crop: project::Crop,
    pub fit: Fit,
    pub opacity: f32,
}
impl Default for Placement {
    fn default() -> Self {
        Self {
            destination: Rect::default(),
            crop: project::Crop::default(),
            fit: Fit::Cover,
            opacity: 1.0,
        }
    }
}
#[derive(Clone)]
pub struct RenderSettings {
    pub width: u32,
    pub height: u32,
    pub effects: Option<Arc<dyn crate::effects::EffectCompositor>>,
    pub placements: BTreeMap<ItemId, Placement>,
    pub background_blur: Option<u32>,
    pub text_cache_bytes: u64,
    pub text_cache_entries: usize,
    pub caption_languages: Vec<captions::LanguageTag>,
    pub caption_style: CaptionStyle,
}
impl RenderSettings {
    pub fn from_project(project: &Project) -> Self {
        Self {
            width: project.data().canvas.width,
            height: project.data().canvas.height,
            effects: None,
            placements: BTreeMap::new(),
            background_blur: None,
            text_cache_bytes: 8 << 20,
            text_cache_entries: 4,
            caption_languages: Vec::new(),
            caption_style: CaptionStyle::default(),
        }
    }
    pub fn portrait(project: &Project) -> Self {
        let mut value = Self::from_project(project);
        value.width = 1080;
        value.height = 1920;
        value
    }
    fn validate(&self, project: &Project) -> Result<()> {
        Image::bytes(self.width, self.height)?;
        caption::validate(&self.caption_style)?;
        if self.background_blur.is_some_and(|r| r > 64)
            || self.placements.len() > 100000
            || self.text_cache_bytes > 256 << 20
            || self.text_cache_entries > 64
            || self.caption_languages.len() > 2
            || (self.caption_languages.len() == 2
                && self.caption_languages.first() == self.caption_languages.get(1))
        {
            return Err(Error::Invalid("render settings limits"));
        }
        for (id, p) in &self.placements {
            p.destination.validate()?;
            if !p.crop.valid() {
                return Err(Error::Invalid("normalized rectangle"));
            }
            if !p.opacity.is_finite()
                || !(0.0..=1.0).contains(&p.opacity)
                || !project
                    .data()
                    .scene()
                    .timeline
                    .iter()
                    .any(|t| t.items.iter().any(|i| i.id == *id))
            {
                return Err(Error::Invalid("item placement"));
            }
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RasterCacheStats {
    pub entries: usize,
    pub bytes: u64,
    pub hits: u64,
    pub misses: u64,
}

pub struct Renderer<'a> {
    project: &'a Project,
    settings: RenderSettings,
    text_cache: BTreeMap<TextKey, TextEntry>,
    cache_stats: RasterCacheStats,
    cache_clock: u64,
    opacity_path: AnimationPath,
    /// Sticker assets the last frames could not draw; each failure is kept once.
    sticker_errors: Vec<StickerError>,
}
