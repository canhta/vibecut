//! Evaluates project items into the rendered frame.

use super::draw::{blur, draw, horizontal_window};
use super::raster::{TextBox, TextEntry, TextFace, TextKey, raster_text};
use super::{Fit, Placement, RasterCacheStats, Rect, RenderSettings, Renderer};
use crate::{
    CaptionPosition, CaptionStyle, Error, ExecutionHooks, Image, MemoryKind, Result,
    SourceProvider, Unavailable, check,
    image::{color, over},
};
use project::{AnimationPath, BuiltinPath, Item, ItemParams, ParamValue, Project};
use std::collections::BTreeMap;
use std::sync::Arc;
use time::Tick;

impl<'a> Renderer<'a> {
    pub fn new(project: &'a Project, settings: RenderSettings) -> Result<Self> {
        settings.validate(project)?;
        Ok(Self {
            project,
            settings,
            text_cache: BTreeMap::new(),
            cache_stats: RasterCacheStats::default(),
            cache_clock: 0,
            opacity_path: AnimationPath::from(BuiltinPath::Opacity),
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
    pub(super) fn placement_at(&self, item: &Item, local: Tick) -> Result<Placement> {
        let mut placement = self.placement(item, local)?;
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
    fn placement(&self, item: &Item, local: Tick) -> Result<Placement> {
        let placement = self
            .settings
            .placements
            .get(&item.id)
            .copied()
            .unwrap_or_else(|| {
                if matches!(item.content, ItemParams::Caption(_)) {
                    self.settings.caption_style.cue_placement(None)
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
            });
        item_crop(item, placement, local)
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
        let timeline = &project.data().scene().timeline;
        let transitions: BTreeMap<_, _> = timeline
            .transitions
            .iter()
            .filter(|tr| tr.contains(at))
            .map(|tr| (tr.track, tr))
            .collect();
        let mut active_transitions = BTreeMap::new();
        let mut active = Vec::new();
        for track in timeline.iter().filter(|t| !t.hidden) {
            if let Some(tr) = transitions.get(&track.id) {
                let (a, b) = tr.contributors(timeline, at)?;
                active.extend([a, b]);
                active_transitions.insert(a.id, (*tr, true));
                active_transitions.insert(b.id, (*tr, false));
            } else {
                active.extend(
                    track
                        .items
                        .iter()
                        .filter(|i| i.start <= at && i.end().is_ok_and(|end| at < end)),
                );
            }
        }
        if active.len() > 4096 {
            return Err(Error::Invalid("frame layer expansion"));
        }
        active.retain(|item| !item.hidden);
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
                let placement = item_crop(item, Placement::default(), local_time(item, at)?)?;
                if let Some(image) = self.video_image(
                    item,
                    source_time(item, at)?,
                    placement,
                    sources,
                    hooks,
                    preview,
                )? {
                    draw(&mut out, &image, placement, hooks)?;
                    blur(&mut out, radius, hooks)?;
                    prepared = Some((item.id, image));
                    break;
                }
            }
        }
        let mut transition_base = None;
        let mut outgoing = None;
        for item in active {
            check(hooks)?;
            let transition = active_transitions.get(&item.id).copied();
            if transition.is_some_and(|(_, outgoing)| outgoing) {
                transition_base = Some(out.copy(hooks)?);
            }
            let local = local_time(item, at)?;
            let mut placement = self.placement_at(item, local)?;
            if placement.opacity != 0.0 {
                let masked = item.masks.iter().any(|m| m.active());
                let transform = super::transform::Transform::at(item, local)?;
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
                let final_opacity = if masked || blended || !passes.is_empty() || transform.active()
                {
                    let opacity = placement.opacity;
                    placement.opacity = 1.;
                    opacity
                } else {
                    1.
                };
                let below = if passes.is_empty() && !masked && !blended && !transform.active() {
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
                        let stroke = color(
                            crate::color::at(
                                &item.animations,
                                &AnimationPath::new("params.strokeColor")?,
                                local,
                                project::Color([0; 4]),
                            )?
                            .0,
                        );
                        let stroke_width =
                            2.0 * f64::from(out.width) / f64::from(project.data().canvas.width);
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
                                if stroke[3] > 0.0 {
                                    let edge = (x - r.x * f64::from(w))
                                        .min((r.x + r.width) * f64::from(w) - x)
                                        .min(y - r.y * f64::from(h))
                                        .min((r.y + r.height) * f64::from(h) - y);
                                    if edge < stroke_width {
                                        over(pixel, stroke, a);
                                    }
                                }
                            }
                        }
                    }
                    ItemParams::Text(p) => {
                        let scale =
                            self.settings.width as f32 / self.project.data().canvas.width as f32;
                        // Style animations share the item clock; unkeyed fields keep the stored value.
                        let fill = crate::color::at(
                            &item.animations,
                            &AnimationPath::from(BuiltinPath::Color),
                            local,
                            p.color,
                        )?;
                        let background = match &p.background {
                            None => None,
                            Some(b) => {
                                let scalar = |path: BuiltinPath, base: f32| -> Result<f32> {
                                    scalar_at(&item.animations, path, local, base)
                                };
                                Some(TextBox {
                                    padding_x: scalar(
                                        BuiltinPath::BackgroundPaddingX,
                                        b.padding_x,
                                    )?
                                    .clamp(0.0, project::TextBackground::MAX_EXTENT)
                                        * scale,
                                    padding_y: scalar(
                                        BuiltinPath::BackgroundPaddingY,
                                        b.padding_y,
                                    )?
                                    .clamp(0.0, project::TextBackground::MAX_EXTENT)
                                        * scale,
                                    offset_x: scalar(BuiltinPath::BackgroundOffsetX, b.offset_x)?
                                        .clamp(
                                            -project::TextBackground::MAX_OFFSET,
                                            project::TextBackground::MAX_OFFSET,
                                        )
                                        * scale,
                                    offset_y: scalar(BuiltinPath::BackgroundOffsetY, b.offset_y)?
                                        .clamp(
                                            -project::TextBackground::MAX_OFFSET,
                                            project::TextBackground::MAX_OFFSET,
                                        )
                                        * scale,
                                    radius: scalar(BuiltinPath::BackgroundCornerRadius, b.radius)?
                                        .clamp(0.0, project::TextBackground::MAX_EXTENT)
                                        * scale,
                                    color: crate::color::at(
                                        &item.animations,
                                        &AnimationPath::from(BuiltinPath::BackgroundColor),
                                        local,
                                        b.color,
                                    )?
                                    .0,
                                })
                            }
                        };
                        let style = text::style_name(p.weight, p.italic);
                        // The raster grows by the box margin on every side; the text keeps its place.
                        let margin = f64::from(background.map_or(0.0, TextBox::margin));
                        let mut grown = placement;
                        grown.destination = Rect {
                            x: placement.destination.x - margin / f64::from(self.settings.width),
                            y: placement.destination.y - margin / f64::from(self.settings.height),
                            width: placement.destination.width
                                + 2.0 * margin / f64::from(self.settings.width),
                            height: placement.destination.height
                                + 2.0 * margin / f64::from(self.settings.height),
                        };
                        let image = self.text(
                            p.content.as_str(),
                            p.font_size,
                            fill.0,
                            grown,
                            None,
                            TextFace {
                                family: p.font.as_str(),
                                style: &style,
                                letter_spacing: p.letter_spacing,
                                line_height: p.line_height,
                                align: p.align,
                                decoration: p.decoration,
                            },
                            background,
                            hooks,
                        )?;
                        draw(&mut out, &image, grown, hooks)?;
                    }
                    ItemParams::Caption(p) => {
                        for cue in
                            p.cues.cues.iter().filter(|c| {
                                c.range.start() <= source_at && source_at < c.range.end()
                            })
                        {
                            let mut selected: Vec<_> = if self.settings.caption_languages.is_empty()
                            {
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
                                let _joined =
                                    hooks.reserve(MemoryKind::Compositor, bytes as u64)?;
                                let mut value = String::new();
                                value.try_reserve_exact(bytes)?;
                                for text in selected {
                                    if !value.is_empty() {
                                        value.push('\n');
                                    }
                                    value.push_str(text.text.as_str());
                                }
                                let cue_style = p.cues.style_of(cue);
                                let style = self.settings.caption_style.for_cue(cue_style);
                                let caption_placement = style.cue_placement(cue_style);
                                let scale = self.settings.height as f32 / 1080.0;
                                let background = style.background.as_ref().map(|b| {
                                    TextBox::uniform(b.padding * scale, b.radius * scale, b.color)
                                });
                                let image = self.text(
                                    &value,
                                    style.size,
                                    style.color,
                                    caption_placement,
                                    Some(&style),
                                    TextFace {
                                        family: style.font.as_str(),
                                        style: style.font_style.as_str(),
                                        align: super::caption::align(cue_style),
                                        decoration: super::caption::decoration(cue_style),
                                        ..Default::default()
                                    },
                                    background,
                                    hooks,
                                )?;
                                draw(&mut out, &image, caption_placement, hooks)?;
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
                            Err(Error::Unsupported(_)) => {
                                cpu.apply(&out, &passes, inputs(), hooks)?
                            }
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
                            |mask, size, geometry| {
                                let canvas_height = project.data().canvas.height as f32;
                                let face_style = text::style_name(mask.weight, mask.italic);
                                let style = CaptionStyle {
                                    font_style: face_style.clone(),
                                    outline: None,
                                    background: None,
                                    position: CaptionPosition::Middle,
                                    max_lines: 64,
                                    ..Default::default()
                                };
                                self.text(
                                    mask.content,
                                    size * 1080.0 / canvas_height,
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
                                    TextFace {
                                        family: mask.font,
                                        style: &face_style,
                                        letter_spacing: mask.letter_spacing * 1080.0
                                            / canvas_height,
                                        line_height: mask.line_height * 1080.0 / canvas_height,
                                        ..Default::default()
                                    },
                                    None,
                                    hooks,
                                )
                            },
                            hooks,
                        )?;
                    }
                    let cpu = CpuEffects;
                    let adapter = self.settings.effects.as_deref().unwrap_or(&cpu);
                    if transform.active() {
                        let r = placement.destination;
                        filtered = transform.apply(
                            &filtered,
                            &project.data().canvas,
                            [r.x + r.width * 0.5, r.y + r.height * 0.5],
                            hooks,
                        )?;
                    }
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
            if let Some((tr, is_outgoing)) = transition {
                if is_outgoing {
                    let base = transition_base
                        .take()
                        .ok_or(Error::Invalid("transition backdrop"))?;
                    outgoing = Some(std::mem::replace(&mut out, base));
                } else {
                    let a = outgoing
                        .take()
                        .ok_or(Error::Invalid("transition outgoing frame"))?;
                    let elapsed = at
                        .checked_sub(tr.start)
                        .ok_or(Error::Invalid("transition time"))?;
                    let progress = elapsed.0 as f64 / tr.duration.0 as f64;
                    let canvas = self.project.data().canvas.width.max(1);
                    out = super::transitions::apply(
                        &tr.kind,
                        &tr.params,
                        &a,
                        &out,
                        progress,
                        self.settings.width as f32 / canvas as f32,
                        hooks,
                    )?;
                }
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
        face: TextFace<'_>,
        background: Option<TextBox>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Arc<Image>> {
        let font = face.family;
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
        let font_style = caption.map_or(face.style, |s| s.font_style.as_str());
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
            letter_spacing: (face.letter_spacing * scale).to_bits(),
            line_height: (face.line_height * scale).to_bits(),
            fonts: text::generation(),
            outline: caption
                .and_then(|s| s.outline.as_ref())
                .map(|o| ((o.width * scale).to_bits(), o.color)),
            background: background.map(TextBox::key),
            align: face.align,
            decoration: face.decoration,
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
/// Maps the item's source-normalized crop into the placement's source window so
/// sampling keeps the cropped region and every later stage sees the placed item.
/// The crop position follows the `crop.x` / `crop.y` channels at the item-local
/// time, clamped into the frame by `project::Crop::at`; preview and export both
/// reach it through `placement_at`.
fn item_crop(item: &Item, mut placement: Placement, local: Tick) -> Result<Placement> {
    if let ItemParams::Video(p) = &item.content
        && let Some(crop) = p.crop
    {
        let crop = crop.at(&item.animations, local)?;
        let base = placement.crop;
        placement.crop = project::Crop {
            x: base.x + crop.x * base.width,
            y: base.y + crop.y * base.height,
            width: crop.width * base.width,
            height: crop.height * base.height,
        };
    }
    Ok(placement)
}
/// Finite scalar channel value at the item-local time, or the stored base when unkeyed.
fn scalar_at(
    animations: &project::Animations,
    path: BuiltinPath,
    at: Tick,
    base: f32,
) -> Result<f32> {
    match animations.value_at(&AnimationPath::from(path), at)? {
        None => Ok(base),
        Some(ParamValue::Scalar(v)) if v.is_finite() => Ok(v as f32),
        _ => Err(Error::Invalid("text style animation value")),
    }
}
fn source_time(item: &Item, at: Tick) -> Result<Tick> {
    // Project owns the one retime-aware mapping: nearest earlier source frame at non-1x speed.
    item.source_time_at(at)
        .map_err(|_| Error::Invalid("item source overflow"))
}
fn local_time(item: &Item, at: Tick) -> Result<Tick> {
    at.checked_sub(item.start)
        .ok_or(Error::Invalid("item source mapping"))
}
