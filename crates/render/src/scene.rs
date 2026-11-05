use crate::{
    CaptionPosition, CaptionStyle, Error, ExecutionHooks, Image, MemoryKind, Result,
    SourceProvider, Unavailable, check,
    image::{color, over},
};
use project::{AnimationPath, Item, ItemId, ItemParams, ParamValue, Project};
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
    /// Validate a nonempty source-normalized crop contained in `[0, 1]².
    /// Non-finite values and out-of-source rectangles return Invalid.
    pub fn validate_crop(self) -> Result<()> {
        self.validate(true)
    }
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
        self.caption_style.validate()?;
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
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct TextKey {
    value: String,
    size: u32,
    fill: [u8; 4],
    width: u32,
    height: u32,
    font: String,
    style: String,
    outline: Option<(u32, [u8; 4])>,
    background: Option<(u32, u32, [u8; 4])>,
    position: CaptionPosition,
    max_lines: Option<usize>,
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
    opacity_path: AnimationPath,
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
            opacity_path: AnimationPath::new("opacity")
                .map_err(|_| Error::Invalid("opacity animation path"))?,
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
    /// Static placement with project-owned keyframed opacity applied at `local`.
    fn placement_at(&self, item: &Item, local: Tick) -> Result<Placement> {
        let mut placement = self.placement(item);
        let Some(value) = item.animations.value_at(&self.opacity_path, local)? else {
            return Ok(placement);
        };
        let ParamValue::Scalar(opacity) = value else {
            return Err(Error::Invalid("opacity animation value"));
        };
        if !opacity.is_finite() {
            return Err(Error::Invalid("opacity animation value"));
        }
        // The compositor's source-over contract admits only 0..=1 opacity.
        placement.opacity = opacity.clamp(0.0, 1.0) as f32;
        Ok(placement)
    }
    fn placement(&self, item: &Item) -> Placement {
        self.settings
            .placements
            .get(&item.id)
            .copied()
            .unwrap_or_else(|| {
                if matches!(item.content, ItemParams::Caption(_)) {
                    let style = &self.settings.caption_style;
                    let inset = f64::from(style.safe_area_percent) / 100.0;
                    let decoration = style
                        .outline
                        .as_ref()
                        .map_or(0.0, |o| o.width)
                        .max(style.background.as_ref().map_or(0.0, |b| b.padding));
                    let height = ((f64::from(style.size) * style.max_lines as f64 * 2.0
                        + f64::from(decoration) * 2.0
                        + 4.0)
                        / 1080.0)
                        .min(1.0 - 2.0 * inset);
                    let y = match style.position {
                        CaptionPosition::Top => inset,
                        CaptionPosition::Middle => (1.0 - height) / 2.0,
                        CaptionPosition::Bottom => 1.0 - inset - height,
                    };
                    Placement {
                        destination: Rect {
                            x: inset,
                            y,
                            width: 1.0 - 2.0 * inset,
                            height,
                        },
                        fit: Fit::Stretch,
                        ..Default::default()
                    }
                } else if let ItemParams::Text(params) = &item.content {
                    let width = 0.8;
                    let height = 0.25;
                    let x = f64::from(params.position.x) / 100.0 - width / 2.0;
                    let y = f64::from(params.position.y) / 100.0 - height / 2.0;
                    Placement {
                        destination: Rect {
                            x,
                            y,
                            width,
                            height,
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
            .scene()
            .timeline
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
                if self.placement_at(item, local_time(item, at)?)?.opacity == 0.0 {
                    continue;
                }
                if let Some(image) = self.video_image(
                    item,
                    source_time(item, at)?,
                    Placement::default(),
                    sources,
                    hooks,
                    preview,
                )? {
                    draw(&mut out, &image, Placement::default(), hooks)?;
                    blur(&mut out, radius, hooks)?;
                    prepared = Some((item.id, image));
                    break;
                }
            }
        }
        for item in active {
            check(hooks)?;
            let local = local_time(item, at)?;
            let mut placement = self.placement_at(item, local)?;
            if placement.opacity == 0.0 {
                continue;
            }
            let masked = item.masks.iter().any(|m| m.active());
            let blended = item.blend_mode != project::BlendMode::Normal;
            let source_at = source_time(item, at)?;
            let _plan = if item.effects.iter().any(|effect| effect.enabled) {
                Some(hooks.reserve(
                    MemoryKind::Compositor,
                    item.effects.len() as u64
                        * 512
                        * (std::mem::size_of::<crate::effects::EffectPass>() as u64 + 49 * 4),
                )?)
            } else {
                None
            };
            let passes = crate::effects::evaluate(item, local, out.width, out.height)?;
            let final_opacity = if masked || blended || !passes.is_empty() {
                let opacity = placement.opacity;
                placement.opacity = 1.;
                opacity
            } else {
                1.
            };
            let below = if passes.is_empty() && !masked && !blended {
                None
            } else {
                let layer = Image::new(out.width, out.height, [0.0; 4], hooks)?;
                Some(std::mem::replace(&mut out, layer))
            };
            match &item.content {
                ItemParams::Video(_) => {
                    let image = if prepared.as_ref().is_some_and(|(id, _)| *id == item.id) {
                        prepared.take().map(|(_, image)| image)
                    } else {
                        self.video_image(item, source_at, placement, sources, hooks, preview)?
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
                    let scale =
                        self.settings.width as f32 / self.project.data().canvas.width as f32;
                    let background = p
                        .background
                        .as_ref()
                        .map(|b| (b.padding * scale, b.radius * scale, b.color.0));
                    let image = self.text(
                        p.content.as_str(),
                        p.font_size,
                        p.color.0,
                        placement,
                        None,
                        p.font.as_str(),
                        background,
                        hooks,
                    )?;
                    draw(&mut out, &image, placement, hooks)?;
                }
                ItemParams::Caption(p) => {
                    for cue in p
                        .cues
                        .cues
                        .iter()
                        .filter(|c| c.range.start() <= source_at && source_at < c.range.end())
                    {
                        let mut selected: Vec<_> = if self.settings.caption_languages.is_empty() {
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
                        selected.retain(|text| !text.text.as_str().trim().is_empty());
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
                            let style = self.settings.caption_style.clone();
                            let scale = self.settings.height as f32 / 1080.0;
                            let background = style
                                .background
                                .as_ref()
                                .map(|b| (b.padding * scale, b.radius * scale, b.color));
                            let image = self.text(
                                &value,
                                style.size,
                                style.color,
                                placement,
                                Some(&style),
                                style.font.as_str(),
                                background,
                                hooks,
                            )?;
                            draw(&mut out, &image, placement, hooks)?;
                        }
                    }
                }
                ItemParams::Audio(_) => {}
            }
            if let Some(mut below) = below {
                use crate::effects::{CpuEffects, EffectCompositor, EffectInputs};
                let inputs = || EffectInputs {
                    coverage: None,
                    below: &below,
                };
                let cpu = CpuEffects;
                let adapter = self.settings.effects.as_deref().unwrap_or(&cpu);
                let mut filtered = if passes.is_empty() {
                    out
                } else {
                    match adapter.apply(&out, &passes, inputs(), hooks) {
                        Err(Error::Unsupported(_)) => cpu.apply(&out, &passes, inputs(), hooks)?,
                        result => result?,
                    }
                };
                if filtered.width != below.width || filtered.height != below.height {
                    return Err(Error::Invalid("effect output dimensions"));
                }
                if masked {
                    let width = self.settings.width;
                    let height = self.settings.height;
                    crate::mask::apply(
                        &mut filtered,
                        item,
                        local,
                        placement,
                        &project.data().canvas,
                        |content, size, geometry| {
                            let style = CaptionStyle {
                                font_style: "Regular".into(),
                                outline: None,
                                background: None,
                                position: CaptionPosition::Middle,
                                max_lines: 64,
                                ..Default::default()
                            };
                            self.text(
                                content,
                                size * 1080.0 / project.data().canvas.height as f32,
                                [255; 4],
                                Placement {
                                    destination: Rect {
                                        x: 0.,
                                        y: 0.,
                                        width: f64::from(geometry.size[0]) / f64::from(width),
                                        height: f64::from(geometry.size[1]) / f64::from(height),
                                    },
                                    ..Default::default()
                                },
                                Some(&style),
                                "Inter",
                                None,
                                hooks,
                            )
                        },
                        hooks,
                    )?;
                }
                let cpu = CpuEffects;
                let adapter = self.settings.effects.as_deref().unwrap_or(&cpu);
                match adapter.composite(
                    &mut below,
                    &filtered,
                    final_opacity,
                    item.blend_mode,
                    hooks,
                ) {
                    Err(Error::Unsupported(_)) => crate::blend::composite(
                        &mut below,
                        &filtered,
                        final_opacity,
                        item.blend_mode,
                        hooks,
                    )?,
                    result => result?,
                }
                out = below;
            }
        }
        Ok(out)
    }
    #[allow(clippy::too_many_arguments)]
    fn video_image(
        &self,
        item: &Item,
        at: Tick,
        placement: Placement,
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
        let window = if self.settings.background_blur.is_none() {
            asset
                .probe()
                .streams
                .iter()
                .find_map(|s| s.video.as_ref())
                .map(|shape| {
                    horizontal_window(
                        self.settings.width,
                        self.settings.height,
                        shape.width,
                        shape.height,
                        placement,
                    )
                })
                .transpose()?
                .flatten()
        } else {
            None
        };
        sources.video_window(asset, item.id, at, window, hooks)
    }
    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        value: &str,
        size: f32,
        fill: [u8; 4],
        placement: Placement,
        caption: Option<&CaptionStyle>,
        font: &str,
        background: Option<(f32, f32, [u8; 4])>,
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
        let scale = if caption.is_some() {
            self.settings.height as f32 / 1080.0
        } else {
            self.settings.width as f32 / self.project.data().canvas.width as f32
        };
        let size = size * scale;
        let font_style = caption.map_or("Regular", |s| s.font_style.as_str());
        let key_bytes = (value.len() + font.len() + font_style.len()) as u64 + 512;
        let key_lease = match hooks.reserve(MemoryKind::Compositor, key_bytes) {
            Err(Error::Budget { .. }) if !self.text_cache.is_empty() => {
                self.clear_text_cache();
                hooks.reserve(MemoryKind::Compositor, key_bytes)?
            }
            result => result?,
        };
        let key = TextKey {
            value: value.to_owned(),
            size: size.to_bits(),
            fill,
            width: w,
            height: h,
            font: font.into(),
            style: font_style.into(),
            outline: caption
                .and_then(|s| s.outline.as_ref())
                .map(|o| ((o.width * scale).to_bits(), o.color)),
            background: background
                .map(|(padding, radius, color)| (padding.to_bits(), radius.to_bits(), color)),
            position: caption.map_or(CaptionPosition::Top, |s| s.position),
            max_lines: caption.map(|s| s.max_lines),
        };
        self.cache_clock = self.cache_clock.saturating_add(1);
        if let Some(entry) = self.text_cache.get_mut(&key) {
            entry.stamp = self.cache_clock;
            self.cache_stats.hits = self.cache_stats.hits.saturating_add(1);
            return Ok(entry.image.clone());
        }
        self.cache_stats.misses = self.cache_stats.misses.saturating_add(1);
        let image = match raster_text(&key, hooks) {
            Err(Error::Budget { .. }) if !self.text_cache.is_empty() => {
                self.clear_text_cache();
                raster_text(&key, hooks)?
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
        .checked_add(local_time(item, at)?)
        .ok_or(Error::Invalid("item source overflow"))
}
fn local_time(item: &Item, at: Tick) -> Result<Tick> {
    at.checked_sub(item.start)
        .ok_or(Error::Invalid("item source mapping"))
}
fn raster_text(key: &TextKey, hooks: &dyn ExecutionHooks) -> Result<Image> {
    let (w, h, size) = (key.width, key.height, f32::from_bits(key.size));
    if !size.is_finite() || size <= 0.0 || size > 4096.0 {
        return Err(Error::Invalid("scaled font size"));
    }
    let bytes = Image::bytes(w, h)?;
    let outline = key.outline.map(|(width, color)| text::Outline {
        width: f32::from_bits(width),
        color: rgba(color),
        kind: text::StrokeKind::Outer,
    });
    let background = key
        .background
        .map(|(padding, radius, color)| text::Rectangle {
            padding: f32::from_bits(padding),
            radius: f32::from_bits(radius),
            color: rgba(color),
        });
    let pad = outline
        .map_or(0.0, |o| o.width)
        .max(background.map_or(0.0, |b| b.padding))
        .ceil();
    if w as f32 <= pad * 2.0 || h as f32 <= pad * 2.0 {
        return Err(Error::Invalid("caption decoration exceeds destination"));
    }
    let _scratch = hooks.reserve(
        MemoryKind::Compositor,
        bytes * 2 + key.value.len() as u64 * 512 + (f64::from(size).powi(2) * 32.0).ceil() as u64,
    )?;
    let mut options = text::LayoutOptions {
        family: &key.font,
        style: &key.style,
        size,
        max_width: w as f32 - pad * 2.0,
        align: text::Align::Center,
        max_lines: key.max_lines,
        ..Default::default()
    };
    let fits = |layout: &text::Layout| {
        let b = layout.bounds;
        !layout.overflow
            && b.x0 >= 0.0
            && b.x1 <= options.max_width
            && b.y1 - b.y0 <= h as f32 - pad * 2.0
    };
    let mut layout = text::layout(&key.value, &options)?;
    if key.max_lines.is_some() && !fits(&layout) {
        // Keep already-fitting captions pixel-identical. Only overflow pays for
        // bounded fitting; cache the final raster under the requested style.
        let mut low = size.min(1.0);
        let mut high = size;
        options.size = low;
        layout = text::layout(&key.value, &options)?;
        if !fits(&layout) {
            return Err(Error::Unsupported(
                "caption exceeds line or destination limits at minimum font size",
            ));
        }
        for _ in 0..10 {
            check(hooks)?;
            options.size = (low + high) / 2.0;
            let candidate = text::layout(&key.value, &options)?;
            if fits(&candidate) {
                low = options.size;
                layout = candidate;
            } else {
                high = options.size;
            }
        }
    }
    let b = layout.bounds;
    let y = if key.max_lines.is_none() {
        0.0
    } else {
        match key.position {
            CaptionPosition::Top => pad - b.y0,
            CaptionPosition::Middle => (h as f32 - (b.y1 - b.y0)) / 2.0 - b.y0,
            CaptionPosition::Bottom => h as f32 - pad - b.y1,
        }
    };
    check(hooks)?;
    let image = text::render(
        &layout,
        text::Canvas {
            width: w,
            height: h,
        },
        (pad, y),
        &text::Paint {
            fill: rgba(key.fill),
            outline,
            background,
        },
    )?;
    Image::from_rgba8(w, h, &image.pixels, true, 13, hooks)
}
fn rgba(value: [u8; 4]) -> text::Color {
    let [r, g, b, a] = value;
    text::Color::rgba(r, g, b, a)
}

struct Geometry {
    dx: f64,
    dy: f64,
    dw: f64,
    dh: f64,
    sx: f64,
    sy: f64,
    sw: f64,
    sh: f64,
}
impl Geometry {
    fn new(
        target_width: u32,
        target_height: u32,
        source_width: u32,
        source_height: u32,
        placement: Placement,
    ) -> Result<Self> {
        let mut dx = placement.destination.x * f64::from(target_width);
        let mut dy = placement.destination.y * f64::from(target_height);
        let mut dw = placement.destination.width * f64::from(target_width);
        let mut dh = placement.destination.height * f64::from(target_height);
        let mut sx = placement.crop.x * f64::from(source_width);
        let mut sy = placement.crop.y * f64::from(source_height);
        let mut sw = placement.crop.width * f64::from(source_width);
        let mut sh = placement.crop.height * f64::from(source_height);
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
        Ok(Self {
            dx,
            dy,
            dw,
            dh,
            sx,
            sy,
            sw,
            sh,
        })
    }
}
fn horizontal_window(
    target_width: u32,
    target_height: u32,
    source_width: u32,
    source_height: u32,
    placement: Placement,
) -> Result<Option<std::ops::Range<u32>>> {
    let sx = f64::from(target_width) * placement.destination.width
        / (f64::from(source_width) * placement.crop.width);
    let sy = f64::from(target_height) * placement.destination.height
        / (f64::from(source_height) * placement.crop.height);
    let scale = match placement.fit {
        Fit::Cover => sx.max(sy),
        _ => sx.min(sy),
    };
    // Half reduction reads neighbors outside this level's sampling footprint.
    if scale <= 0.5 && source_width >= 4 && source_height >= 4 {
        return Ok(None);
    }
    let geometry = Geometry::new(
        target_width,
        target_height,
        source_width,
        source_height,
        placement,
    )?;
    let start = (geometry.sx.floor() - 2.0).clamp(0.0, f64::from(source_width)) as u32;
    let end = ((geometry.sx + geometry.sw).ceil() + 2.0).clamp(0.0, f64::from(source_width)) as u32;
    Ok((start < end && end - start < source_width / 2).then_some(start..end))
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
    let Geometry {
        dx,
        dy,
        dw,
        dh,
        sx,
        sy,
        sw,
        sh,
    } = Geometry::new(
        target.width,
        target.height,
        image.width,
        image.height,
        placement,
    )?;
    let x0 = dx.floor().clamp(0.0, f64::from(target.width)) as usize;
    let x1 = (dx + dw).ceil().clamp(0.0, f64::from(target.width)) as usize;
    let y0 = dy.floor().clamp(0.0, f64::from(target.height)) as usize;
    let y1 = (dy + dh).ceil().clamp(0.0, f64::from(target.height)) as usize;
    // The separable transform is constant across rows. Compute horizontal
    // coordinates once, and resolve vertical neighbors once per row.
    let _axis = hooks.reserve(
        MemoryKind::Compositor,
        ((x1 - x0) * std::mem::size_of::<Option<(i64, f32)>>()) as u64,
    )?;
    let clamp = |p: f64, start: f64, len: f64| {
        if len < 1.0 {
            start + len * 0.5
        } else {
            p.clamp(start + 0.5, start + len - 0.5)
        }
    };
    let mut columns = crate::vector(x1 - x0, None)?;
    for (x, column) in (x0..x1).zip(&mut columns) {
        let px = x as f64 + 0.5;
        if px >= dx && px < dx + dw {
            let fx = clamp(sx + (px - dx) * sw / dw, sx, sw) - 0.5;
            let ix = fx.floor() as i64;
            *column = Some((ix, (fx - ix as f64) as f32));
        }
    }
    let row = |y: i64| {
        if y < 0 || y >= i64::from(image.height) {
            &[][..]
        } else {
            let start = y as usize * image.width as usize;
            image
                .pixels
                .get(start..start + image.width as usize)
                .unwrap_or(&[])
        }
    };
    let target_width = target.width as usize;
    let target_rows = target
        .pixels
        .get_mut(y0 * target_width..y1 * target_width)
        .ok_or(Error::Invalid("compositor rows"))?;
    crate::image::row_chunks(target_rows, target_width, hooks, |start, target_rows| {
        let _rows = hooks.reserve(
            MemoryKind::Compositor,
            ((x1 - x0) * 2 * std::mem::size_of::<[f32; 4]>()) as u64,
        )?;
        let mut top_samples = crate::vector(x1 - x0, [0.0; 4])?;
        let mut bottom_samples = crate::vector(x1 - x0, [0.0; 4])?;
        let mut top_index = None;
        let mut bottom_index = None;
        for (offset, target_row) in target_rows.chunks_mut(target_width).enumerate() {
            let y = y0 + start / target_width + offset;
            check(hooks)?;
            let py = y as f64 + 0.5;
            if py < dy || py >= dy + dh {
                continue;
            }
            let fy = clamp(sy + (py - dy) * sh / dh, sy, sh) - 0.5;
            let iy = fy.floor() as i64;
            let ty = (fy - iy as f64) as f32;
            // Adjacent destination rows usually share a source row. Cache only
            // horizontal interpolation; vertical interpolation keeps its original
            // operation order and rounding for every pixel.
            if bottom_index == Some(iy) {
                std::mem::swap(&mut top_samples, &mut bottom_samples);
                std::mem::swap(&mut top_index, &mut bottom_index);
            }
            if top_index != Some(iy) {
                horizontal_samples(&mut top_samples, row(iy), &columns);
                top_index = Some(iy);
            }
            if bottom_index != Some(iy + 1) {
                horizontal_samples(&mut bottom_samples, row(iy + 1), &columns);
                bottom_index = Some(iy + 1);
            }
            let destination = target_row
                .get_mut(x0..x1)
                .ok_or(Error::Invalid("compositor row"))?;
            for (((pixel, column), top), bottom) in destination
                .iter_mut()
                .zip(&columns)
                .zip(&top_samples)
                .zip(&bottom_samples)
            {
                if column.is_none() {
                    continue;
                }
                let source = std::array::from_fn(|k| {
                    let top = top.get(k).copied().unwrap_or(0.0);
                    let bottom = bottom.get(k).copied().unwrap_or(0.0);
                    top + (bottom - top) * ty
                });
                over(pixel, source, placement.opacity);
            }
        }
        Ok(())
    })?;
    drop(reduced);
    Ok(())
}
fn horizontal_samples(output: &mut [[f32; 4]], row: &[[f32; 4]], columns: &[Option<(i64, f32)>]) {
    for (sample, column) in output.iter_mut().zip(columns) {
        let Some((ix, tx)) = *column else { continue };
        let a = row.get(ix as usize).copied().unwrap_or([0.0; 4]);
        let b = row.get((ix + 1) as usize).copied().unwrap_or([0.0; 4]);
        *sample = std::array::from_fn(|k| {
            let a = a.get(k).copied().unwrap_or(0.0);
            let b = b.get(k).copied().unwrap_or(0.0);
            a + (b - a) * tx
        });
    }
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
