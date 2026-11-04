use crate::{
    Error, ExecutionHooks, Image, MemoryKind, Result, SourceProvider, Unavailable, check,
    image::color,
};
use project::{Item, ItemId, ItemParams, Project};
use std::{collections::BTreeMap, sync::Arc};
use time::Tick;
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
    fn validate(self, crop: bool) -> Result<()> {
        if [self.x, self.y, self.width, self.height]
            .iter()
            .any(|v| !v.is_finite())
            || self.width <= 0.0
            || self.height <= 0.0
            || self.width > 16.0
            || self.height > 16.0
            || self.x.abs() > 16.0
            || self.y.abs() > 16.0
            || (crop
                && (self.x < 0.0
                    || self.y < 0.0
                    || self.x + self.width > 1.0
                    || self.y + self.height > 1.0))
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
    pub crop: Rect,
    pub fit: Fit,
    pub opacity: f32,
}
impl Default for Placement {
    fn default() -> Self {
        Self {
            destination: Rect::default(),
            crop: Rect::default(),
            fit: Fit::Cover,
            opacity: 1.0,
        }
    }
}
#[derive(Clone)]
pub struct RenderSettings {
    pub width: u32,
    pub height: u32,
    pub placements: BTreeMap<ItemId, Placement>,
    pub background_blur: Option<u32>,
    pub text_cache_bytes: u64,
    pub text_cache_entries: usize,
    pub caption_languages: Vec<captions::LanguageTag>,
}
impl RenderSettings {
    pub fn from_project(project: &Project) -> Self {
        Self {
            width: project.data().canvas.width,
            height: project.data().canvas.height,
            placements: BTreeMap::new(),
            background_blur: None,
            text_cache_bytes: 8 << 20,
            text_cache_entries: 4,
            caption_languages: Vec::new(),
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
            p.destination.validate(false)?;
            p.crop.validate(true)?;
            if !p.opacity.is_finite()
                || !(0.0..=1.0).contains(&p.opacity)
                || !project
                    .data()
                    .timeline
                    .tracks
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
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct TextKey {
    value: String,
    size: u32,
    fill: [u8; 4],
    width: u32,
    height: u32,
    font: &'static str,
    style: &'static str,
    fallbacks: &'static str,
}
struct TextEntry {
    image: Arc<Image>,
    stamp: u64,
    bytes: u64,
    _key: Box<dyn crate::Reservation>,
}
pub struct Renderer<'a> {
    project: &'a Project,
    settings: RenderSettings,
    text_cache: BTreeMap<TextKey, TextEntry>,
    cache_stats: RasterCacheStats,
    cache_clock: u64,
}
impl<'a> Renderer<'a> {
    pub fn new(project: &'a Project, settings: RenderSettings) -> Result<Self> {
        settings.validate(project)?;
        Ok(Self {
            project,
            settings,
            text_cache: BTreeMap::new(),
            cache_stats: RasterCacheStats::default(),
            cache_clock: 0,
        })
    }
    pub fn text_cache_stats(&self) -> RasterCacheStats {
        self.cache_stats
    }
    pub fn clear_text_cache(&mut self) {
        self.text_cache.clear();
        self.cache_stats.entries = 0;
        self.cache_stats.bytes = 0;
    }
    /// Strict production frame. Preview placeholders have a separate entry point.
    pub fn render_frame(
        &mut self,
        at: Tick,
        sources: &mut dyn SourceProvider,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Image> {
        self.frame(at, sources, hooks, false)
    }
    pub fn preview_frame(
        &mut self,
        at: Tick,
        sources: &mut dyn SourceProvider,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Image> {
        self.frame(at, sources, hooks, true)
    }
    fn placement(&self, item: &Item) -> Placement {
        self.settings
            .placements
            .get(&item.id)
            .copied()
            .unwrap_or_else(|| {
                if matches!(item.content, ItemParams::Text(_) | ItemParams::Caption(_)) {
                    Placement {
                        destination: Rect {
                            x: 0.1,
                            y: 0.72,
                            width: 0.8,
                            height: 0.25,
                        },
                        fit: Fit::Stretch,
                        ..Default::default()
                    }
                } else {
                    Placement::default()
                }
            })
    }
    fn frame(
        &mut self,
        at: Tick,
        sources: &mut dyn SourceProvider,
        hooks: &dyn ExecutionHooks,
        preview: bool,
    ) -> Result<Image> {
        check(hooks)?;
        if at < Tick::ZERO {
            return Err(Error::Invalid("negative frame time"));
        }
        let project = self.project;
        let active: Vec<_> = project
            .data()
            .timeline
            .tracks
            .iter()
            .filter(|t| !t.hidden)
            .flat_map(|t| &t.items)
            .filter(|i| i.start <= at && i.end().is_ok_and(|end| at < end))
            .collect();
        if active.len() > 4096 {
            return Err(Error::Invalid("frame layer expansion"));
        }
        let mut layers = active.len();
        for item in &active {
            if let ItemParams::Caption(params) = &item.content {
                let source_at = source_time(item, at)?;
                for _ in params
                    .cues
                    .cues
                    .iter()
                    .filter(|c| c.range.contains(source_at))
                {
                    layers += 1;
                    if layers > 4096 {
                        return Err(Error::Invalid("caption layer expansion"));
                    }
                }
            }
        }
        let video: Vec<_> = active
            .iter()
            .filter(|i| matches!(i.content, ItemParams::Video(_)))
            .map(|i| i.id)
            .collect();
        sources.begin_video(&video);
        let output = Image::new(
            self.settings.width,
            self.settings.height,
            color(self.project.data().canvas.background.0),
            hooks,
        );
        let mut out = match output {
            Err(Error::Budget { .. }) if !self.text_cache.is_empty() => {
                self.clear_text_cache();
                Image::new(
                    self.settings.width,
                    self.settings.height,
                    color(self.project.data().canvas.background.0),
                    hooks,
                )?
            }
            result => result?,
        };
        let mut prepared = None;
        if let Some(radius) = self.settings.background_blur {
            for item in active
                .iter()
                .filter(|i| matches!(i.content, ItemParams::Video(_)))
            {
                if let Some(image) =
                    self.video_image(item, source_time(item, at)?, sources, hooks, preview)?
                {
                    draw(&mut out, &image, Placement::default(), hooks)?;
                    blur(&mut out, radius, hooks)?;
                    prepared = Some((item.id, image));
                    break;
                }
            }
        }
        for item in active {
            check(hooks)?;
            let placement = self.placement(item);
            let source_at = source_time(item, at)?;
            match &item.content {
                ItemParams::Video(_) => {
                    let image = if prepared.as_ref().is_some_and(|(id, _)| *id == item.id) {
                        prepared.take().map(|(_, image)| image)
                    } else {
                        self.video_image(item, source_at, sources, hooks, preview)?
                    };
                    if let Some(image) = image {
                        draw(&mut out, &image, placement, hooks)?;
                    }
                }
                ItemParams::Overlay(p) => {
                    let c = color(p.color.0);
                    let a = placement.opacity;
                    let w = out.width;
                    let h = out.height;
                    for (index, pixel) in out.pixels.iter_mut().enumerate() {
                        let x = (index % w as usize) as f64 + 0.5;
                        let y = (index / w as usize) as f64 + 0.5;
                        let r = placement.destination;
                        if x >= r.x * f64::from(w)
                            && x < (r.x + r.width) * f64::from(w)
                            && y >= r.y * f64::from(h)
                            && y < (r.y + r.height) * f64::from(h)
                        {
                            over(pixel, c, a);
                        }
                    }
                }
                ItemParams::Text(p) => {
                    let image =
                        self.text(p.content.as_str(), p.font_size, p.color.0, placement, hooks)?;
                    draw(&mut out, &image, placement, hooks)?;
                }
                ItemParams::Caption(p) => {
                    for cue in p
                        .cues
                        .cues
                        .iter()
                        .filter(|c| c.range.start() <= source_at && source_at < c.range.end())
                    {
                        let selected: Vec<_> = if self.settings.caption_languages.is_empty() {
                            cue.texts
                                .get(&cue.source_lang)
                                .or_else(|| cue.texts.values().next())
                                .into_iter()
                                .collect()
                        } else {
                            self.settings
                                .caption_languages
                                .iter()
                                .map(|language| {
                                    cue.texts.get(language).ok_or(Error::Unsupported(
                                        "requested caption language unavailable",
                                    ))
                                })
                                .collect::<Result<Vec<_>>>()?
                        };
                        let bytes = selected
                            .iter()
                            .try_fold(0usize, |sum, text| {
                                sum.checked_add(text.text.as_str().len() + 1)
                            })
                            .ok_or(Error::Invalid("caption text bytes"))?;
                        if bytes > 16384 {
                            return Err(Error::Invalid("render caption byte limit"));
                        }
                        if !selected.is_empty() {
                            let _joined = hooks.reserve(MemoryKind::Compositor, bytes as u64)?;
                            let mut value = String::new();
                            value.try_reserve_exact(bytes)?;
                            for text in selected {
                                if !value.is_empty() {
                                    value.push('\n');
                                }
                                value.push_str(text.text.as_str());
                            }
                            let size = self.project.data().canvas.height as f32 * 0.06;
                            let image = self.text(&value, size, [255; 4], placement, hooks)?;
                            draw(&mut out, &image, placement, hooks)?;
                        }
                    }
                }
                ItemParams::Audio(_) => {}
            }
        }
        Ok(out)
    }
    fn video_image(
        &self,
        item: &Item,
        at: Tick,
        sources: &mut dyn SourceProvider,
        hooks: &dyn ExecutionHooks,
        preview: bool,
    ) -> Result<Option<Image>> {
        let asset = self
            .project
            .data()
            .assets
            .iter()
            .find(|a| Some(a.id()) == item.source_asset)
            .ok_or(Error::Invalid("video asset"))?;
        if let Some(reason) = sources.status(asset, false, hooks)? {
            if preview {
                return Ok(None);
            }
            return Err(Error::Unavailable(vec![Unavailable {
                asset: asset.id(),
                item: item.id,
                range: item.source_range,
                reason,
            }]));
        }
        sources.video(asset, item.id, at, hooks)
    }
    fn text(
        &mut self,
        value: &str,
        size: f32,
        fill: [u8; 4],
        placement: Placement,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Arc<Image>> {
        if value.len() > 16384 {
            return Err(Error::Invalid("render text byte limit"));
        }
        let w = (f64::from(self.settings.width) * placement.destination.width)
            .round()
            .max(1.0) as u32;
        let h = (f64::from(self.settings.height) * placement.destination.height)
            .round()
            .max(1.0) as u32;
        let size = size * self.settings.width as f32 / self.project.data().canvas.width as f32;
        let key_bytes = value.len() as u64 + 256;
        let key_lease = match hooks.reserve(MemoryKind::Compositor, key_bytes) {
            Err(Error::Budget { .. }) if !self.text_cache.is_empty() => {
                self.clear_text_cache();
                hooks.reserve(MemoryKind::Compositor, key_bytes)?
            }
            result => result?,
        };
        let defaults = text::LayoutOptions::default();
        let key = TextKey {
            value: value.to_owned(),
            size: size.to_bits(),
            fill,
            width: w,
            height: h,
            font: defaults.family,
            style: defaults.style,
            fallbacks: "bundled-default-v1",
        };
        self.cache_clock = self.cache_clock.saturating_add(1);
        if let Some(entry) = self.text_cache.get_mut(&key) {
            entry.stamp = self.cache_clock;
            self.cache_stats.hits = self.cache_stats.hits.saturating_add(1);
            return Ok(entry.image.clone());
        }
        self.cache_stats.misses = self.cache_stats.misses.saturating_add(1);
        let image = match raster_text(value, size, fill, w, h, hooks) {
            Err(Error::Budget { .. }) if !self.text_cache.is_empty() => {
                self.clear_text_cache();
                raster_text(value, size, fill, w, h, hooks)?
            }
            result => result?,
        };
        let image = Arc::new(image);
        let bytes = Image::bytes(w, h)?
            .checked_add(key_bytes)
            .ok_or(Error::Invalid("text cache bytes"))?;
        if self.settings.text_cache_entries == 0 || bytes > self.settings.text_cache_bytes {
            return Ok(image);
        }
        while self.text_cache.len() >= self.settings.text_cache_entries
            || self.cache_stats.bytes + bytes > self.settings.text_cache_bytes
        {
            let key = self
                .text_cache
                .iter()
                .min_by_key(|(_, e)| e.stamp)
                .map(|(key, _)| key.clone())
                .ok_or(Error::Invalid("text cache eviction"))?;
            if let Some(old) = self.text_cache.remove(&key) {
                self.cache_stats.bytes -= old.bytes;
            }
        }
        self.text_cache.insert(
            key,
            TextEntry {
                image: image.clone(),
                stamp: self.cache_clock,
                bytes,
                _key: key_lease,
            },
        );
        self.cache_stats.bytes += bytes;
        self.cache_stats.entries = self.text_cache.len();
        Ok(image)
    }
}
fn source_time(item: &Item, at: Tick) -> Result<Tick> {
    item.source_range
        .start()
        .checked_add(
            at.checked_sub(item.start)
                .ok_or(Error::Invalid("item source mapping"))?,
        )
        .ok_or(Error::Invalid("item source overflow"))
}
fn raster_text(
    value: &str,
    size: f32,
    fill: [u8; 4],
    w: u32,
    h: u32,
    hooks: &dyn ExecutionHooks,
) -> Result<Image> {
    let bytes = Image::bytes(w, h)?;
    let _scratch = hooks.reserve(
        MemoryKind::Compositor,
        bytes * 2 + value.len() as u64 * 512 + (f64::from(size).powi(2) * 32.0).ceil() as u64,
    )?;
    let layout = text::layout(
        value,
        &text::LayoutOptions {
            size,
            max_width: w as f32,
            align: text::Align::Center,
            ..Default::default()
        },
    )?;
    let image = text::render(
        &layout,
        text::Canvas {
            width: w,
            height: h,
        },
        (0.0, 0.0),
        &text::Paint {
            fill: text::Color::rgba(
                fill.first().copied().unwrap_or(0),
                fill.get(1).copied().unwrap_or(0),
                fill.get(2).copied().unwrap_or(0),
                fill.get(3).copied().unwrap_or(0),
            ),
            ..Default::default()
        },
    )?;
    Image::from_rgba8(w, h, &image.pixels, true, 13, hooks)
}
fn over(destination: &mut [f32; 4], source: [f32; 4], opacity: f32) {
    let inverse = 1.0 - source.get(3).copied().unwrap_or(0.0) * opacity;
    for (d, s) in destination.iter_mut().zip(source) {
        *d = (s * opacity + *d * inverse).clamp(0.0, 1.0);
    }
}
fn draw(
    target: &mut Image,
    source: &Image,
    placement: Placement,
    hooks: &dyn ExecutionHooks,
) -> Result<()> {
    let mut reduced = None;
    let mut image = source;
    loop {
        let sx = f64::from(target.width) * placement.destination.width
            / (f64::from(image.width) * placement.crop.width);
        let sy = f64::from(target.height) * placement.destination.height
            / (f64::from(image.height) * placement.crop.height);
        let scale = match placement.fit {
            Fit::Cover => sx.max(sy),
            Fit::Contain => sx.min(sy),
            Fit::Stretch => sx.min(sy),
        };
        if scale > 0.5 || image.width < 4 || image.height < 4 {
            break;
        }
        let next = image.half(hooks)?;
        reduced = Some(next);
        image = reduced.as_ref().ok_or(Error::Invalid("image reduction"))?;
    }
    let mut dx = placement.destination.x * f64::from(target.width);
    let mut dy = placement.destination.y * f64::from(target.height);
    let mut dw = placement.destination.width * f64::from(target.width);
    let mut dh = placement.destination.height * f64::from(target.height);
    let mut sx = placement.crop.x * f64::from(image.width);
    let mut sy = placement.crop.y * f64::from(image.height);
    let mut sw = placement.crop.width * f64::from(image.width);
    let mut sh = placement.crop.height * f64::from(image.height);
    match placement.fit {
        Fit::Cover => {
            let scale = (dw / sw).max(dh / sh);
            let cw = dw / scale;
            let ch = dh / scale;
            sx += (sw - cw) * 0.5;
            sy += (sh - ch) * 0.5;
            sw = cw;
            sh = ch;
        }
        Fit::Contain => {
            let scale = (dw / sw).min(dh / sh);
            let w = sw * scale;
            let h = sh * scale;
            dx += (dw - w) * 0.5;
            dy += (dh - h) * 0.5;
            dw = w;
            dh = h;
        }
        Fit::Stretch => {}
    }
    if [dx, dy, dw, dh, sx, sy, sw, sh]
        .iter()
        .any(|n| !n.is_finite())
        || dw <= 0.0
        || dh <= 0.0
        || sw <= 0.0
        || sh <= 0.0
    {
        return Err(Error::Invalid("placement transform overflow"));
    }
    let x0 = dx.floor().clamp(0.0, f64::from(target.width)) as usize;
    let x1 = (dx + dw).ceil().clamp(0.0, f64::from(target.width)) as usize;
    let y0 = dy.floor().clamp(0.0, f64::from(target.height)) as usize;
    let y1 = (dy + dh).ceil().clamp(0.0, f64::from(target.height)) as usize;
    for y in y0..y1 {
        check(hooks)?;
        for x in x0..x1 {
            let px = x as f64 + 0.5;
            let py = y as f64 + 0.5;
            if px < dx || py < dy || px >= dx + dw || py >= dy + dh {
                continue;
            }
            let u = sx + (px - dx) * sw / dw;
            let v = sy + (py - dy) * sh / dh;
            let clamp = |p: f64, start: f64, len: f64| {
                if len < 1.0 {
                    start + len * 0.5
                } else {
                    p.clamp(start + 0.5, start + len - 0.5)
                }
            };
            let source = image.sample(clamp(u, sx, sw), clamp(v, sy, sh));
            let pixel = target
                .pixels
                .get_mut(y * target.width as usize + x)
                .ok_or(Error::Invalid("compositor index"))?;
            over(pixel, source, placement.opacity);
        }
    }
    drop(reduced);
    Ok(())
}
fn blur(image: &mut Image, radius: u32, hooks: &dyn ExecutionHooks) -> Result<()> {
    if radius == 0 {
        return Ok(());
    }
    let mut temp = Image::new(image.width, image.height, [0.0; 4], hooks)?;
    let r = i64::from(radius);
    let divisor = (2 * radius + 1) as f32;
    for vertical in [false, true] {
        let (source, dest) = if vertical {
            (&temp, &mut *image)
        } else {
            (&*image, &mut temp)
        };
        let outer = if vertical {
            source.width
        } else {
            source.height
        };
        let inner = if vertical {
            source.height
        } else {
            source.width
        };
        for line in 0..outer {
            check(hooks)?;
            let pixel = |p: i64| {
                let p = p.clamp(0, i64::from(inner) - 1);
                if vertical {
                    source.at(i64::from(line), p)
                } else {
                    source.at(p, i64::from(line))
                }
            };
            let mut sum = [0.0f32; 4];
            for p in -r..=r {
                for (s, v) in sum.iter_mut().zip(pixel(p)) {
                    *s += v;
                }
            }
            for n in 0..inner {
                if n > 0 {
                    for ((s, a), b) in sum
                        .iter_mut()
                        .zip(pixel(i64::from(n) - r - 1))
                        .zip(pixel(i64::from(n) + r))
                    {
                        *s += b - a;
                    }
                }
                let index = if vertical {
                    n as usize * source.width as usize + line as usize
                } else {
                    line as usize * source.width as usize + n as usize
                };
                let value = dest
                    .pixels
                    .get_mut(index)
                    .ok_or(Error::Invalid("blur index"))?;
                let alpha = (sum.get(3).copied().unwrap_or(0.0) / divisor).clamp(0.0, 1.0);
                for (channel, (d, s)) in value.iter_mut().zip(sum).enumerate() {
                    *d = if channel == 3 {
                        alpha
                    } else {
                        (s / divisor).clamp(0.0, alpha)
                    };
                }
            }
        }
    }
    Ok(())
}
