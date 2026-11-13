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

/// Glow halo radius as a fraction of the font size; the reference exposes one switch.
const GLOW_SIZE: f32 = 0.15;
/// Largest glow halo radius in output pixels.
const GLOW_MAX: f32 = 64.0;

/// The placement saved in the project, without transient render overrides:
/// full canvas for video, image, overlay and other placed kinds, the text box
/// centred like them (Transform moves it), and the centred graphic author square. Captions
/// resolve through their style; audio has no box.
pub(super) fn saved_placement(canvas: &project::Canvas, item: &Item) -> Option<Placement> {
    Some(match &item.content {
        ItemParams::Text(_) => {
            let width = 0.8;
            let height = 0.25;
            Placement {
                destination: Rect {
                    x: (1. - width) / 2.,
                    y: (1. - height) / 2.,
                    width,
                    height,
                },
                fit: Fit::Stretch,
                ..Default::default()
            }
        }
        ItemParams::Sticker(params) => {
            // The intrinsic size, shrunk to fit the canvas and never enlarged, centred.
            let scale = (f64::from(canvas.width) / f64::from(params.width))
                .min(f64::from(canvas.height) / f64::from(params.height))
                .min(1.);
            let width = f64::from(params.width) * scale / f64::from(canvas.width);
            let height = f64::from(params.height) * scale / f64::from(canvas.height);
            Placement {
                destination: Rect {
                    x: (1. - width) / 2.,
                    y: (1. - height) / 2.,
                    width,
                    height,
                },
                fit: Fit::Stretch,
                ..Default::default()
            }
        }
        ItemParams::Graphic(_) => {
            let side = 512_u32.min(canvas.width).min(canvas.height);
            let width = f64::from(side) / f64::from(canvas.width);
            let height = f64::from(side) / f64::from(canvas.height);
            Placement {
                destination: Rect {
                    x: (1. - width) / 2.,
                    y: (1. - height) / 2.,
                    width,
                    height,
                },
                fit: Fit::Stretch,
                ..Default::default()
            }
        }
        ItemParams::Video(_) | ItemParams::Overlay(_) => Placement::default(),
        ItemParams::Audio(_) | ItemParams::Caption(_) => return None,
    })
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
            opacity_path: AnimationPath::from(BuiltinPath::Opacity),
            sticker_errors: Vec::new(),
        })
    }
    /// Stickers whose asset was missing or invalid in the frames rendered so far; each paints a
    /// placeholder instead of failing the frame.
    pub fn sticker_errors(&self) -> &[super::StickerError] {
        &self.sticker_errors
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
                if let ItemParams::Caption(params) = &item.content {
                    let style = super::caption::item_style(self.project.data(), params)
                        .unwrap_or_else(|| self.settings.caption_style.clone());
                    super::caption::cue_placement(&style, None)
                } else {
                    saved_placement(&self.project.data().canvas, item).unwrap_or_default()
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
        let lut_resources = super::effects::lut::LutResources::resolve(
            project,
            active.iter().flat_map(|item| item.effects.iter()),
            hooks,
        )?;
        let video: Vec<_> = active
            .iter()
            .filter(|i| matches!(i.content, ItemParams::Video(_)))
            .map(|i| i.id)
            .collect();
        let configured = self.settings.effects.clone();
        let cpu = crate::effects::CpuEffects;
        let original = configured.as_deref().unwrap_or(&cpu);
        let (scoped, busy) = match original.frame(hooks) {
            Ok(scope) => (scope, false),
            Err(Error::Unsupported(_)) => (None, true),
            Err(error) => return Err(error),
        };
        let adapter: &dyn crate::effects::EffectCompositor = if busy {
            &cpu
        } else {
            scoped.as_deref().unwrap_or(original)
        };
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
                let local = local_time(item, at)?;
                if self.placement_at(item, local)?.opacity == 0.0 {
                    continue;
                }
                let content = content_local(item, local, out.width, out.height)?;
                let placement = item_crop(item, Placement::default(), local)?;
                if let Some(image) = self.video_image(
                    item,
                    source_time(item, content_tick(item, content)?)?,
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
            adapter.release_idle()?;
            let transition = active_transitions.get(&item.id).copied();
            if transition.is_some_and(|(_, outgoing)| outgoing) {
                transition_base = Some(out.copy(hooks)?);
            }
            let local = local_time(item, at)?;
            let content = content_local(item, local, out.width, out.height)?;
            let mut placement = self.placement_at(item, local)?;
            if placement.opacity != 0.0 {
                let masked = item.masks.iter().any(|m| m.active());
                let transform = super::transform::Transform::at(item, local)?;
                let blended = item.blend_mode != project::BlendMode::Normal;
                let source_at = source_time(item, content_tick(item, content)?)?;
                // Decode once; spatial author units use this unreduced actual raster.
                let video = if matches!(item.content, ItemParams::Video(_)) {
                    if prepared.as_ref().is_some_and(|(id, _)| *id == item.id) {
                        prepared.take().map(|(_, image)| image)
                    } else {
                        self.video_image(item, source_at, placement, sources, hooks, preview)?
                    }
                } else {
                    None
                };
                let geometry = if item.effects.iter().any(|e| e.enabled) {
                    let working = [out.width, out.height];
                    let gains = if matches!(item.content, ItemParams::Video(_)) {
                        let shape = video
                            .as_ref()
                            .map(|image| [image.width(), image.height()])
                            .or_else(|| {
                                project
                                    .data()
                                    .assets
                                    .iter()
                                    .find(|asset| Some(asset.id()) == item.source_asset)
                                    .and_then(|asset| {
                                        asset.probe().streams.iter().find_map(|s| s.video.as_ref())
                                    })
                                    .map(|v| [v.width, v.height])
                            })
                            .ok_or(Error::Invalid("effect spatial geometry"))?;
                        super::draw::clip_pixel_scale(working, shape, placement)?
                    } else {
                        [
                            f64::from(out.width) / f64::from(project.data().canvas.width),
                            f64::from(out.height) / f64::from(project.data().canvas.height),
                        ]
                    };
                    Some(crate::effects::EffectGeometry::new(working, gains)?)
                } else {
                    None
                };
                // Noise reduction is a per-item switch whose pass runs before every
                // colour effect, on this item's own layer.
                let reduce_noise =
                    matches!(&item.content, ItemParams::Video(p) if p.reduce_noise);
                let _plan = if reduce_noise || item.effects.iter().any(|effect| effect.enabled) {
                    Some(hooks.reserve(
                        MemoryKind::Compositor,
                        (item.effects.len() as u64 + u64::from(reduce_noise))
                            * 512
                            * (std::mem::size_of::<crate::effects::EffectPass>() as u64 + 49 * 4),
                    )?)
                } else {
                    None
                };
                // One pass list per effect instance, so a sample op's `input` is the
                // image that instance started with, not the whole item's.
                let mut runs = Vec::new();
                if reduce_noise {
                    runs.push(crate::effects::reduce_noise_passes());
                }
                for effect in item.effects.iter().filter(|effect| effect.enabled) {
                    runs.push(crate::effects::evaluate_effect(
                        &item.animations,
                        effect,
                        local,
                        geometry.ok_or(Error::Invalid("effect spatial geometry"))?,
                        !preview,
                        &lut_resources,
                    )?);
                }
                let has_passes = runs.iter().any(|run| !run.is_empty());
                let graphic = matches!(
                    item.content,
                    ItemParams::Graphic(_) | ItemParams::Sticker(_)
                );
                let final_opacity =
                    if graphic || masked || blended || has_passes || transform.active() {
                        let opacity = placement.opacity;
                        placement.opacity = 1.;
                        opacity
                    } else {
                        1.
                    };
                // The background sits on the canvas under the clip, outside the clip's own
                // layer, so the clip's opacity, blend, transform and effects leave it alone.
                if let ItemParams::Video(params) = &item.content
                    && params.background != project::ClipBackground::None
                    && !super::transform::arrange::covers_canvas(self.project.data(), item, at)?
                {
                    paint_clip_background(
                        &mut out,
                        params.background,
                        video.as_ref(),
                        item,
                        local,
                        hooks,
                    )?;
                }
                let below = if !graphic && !has_passes && !masked && !blended && !transform.active()
                {
                    None
                } else {
                    let layer = Image::new(out.width, out.height, [0.0; 4], hooks)?;
                    Some(std::mem::replace(&mut out, layer))
                };
                match &item.content {
                    ItemParams::Video(_) => {
                        if let Some(image) = video {
                            draw(&mut out, &image, placement, hooks)?;
                        }
                    }
                    ItemParams::Graphic(_) => {
                        super::draw::draw_graphic(&mut out, item, local, placement, hooks)?
                    }
                    ItemParams::Sticker(p) => {
                        if let Some(error) =
                            super::raster::sticker::draw_sticker(&mut out, p, placement, hooks)?
                            && !self.sticker_errors.contains(&error)
                        {
                            self.sticker_errors.push(error);
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
                        // Stroke colour is the one style colour the reference keys.
                        let outline = match &p.stroke {
                            None => None,
                            Some(stroke) => Some((
                                stroke.width * scale,
                                crate::color::at(
                                    &item.animations,
                                    &AnimationPath::from(BuiltinPath::StrokeColor),
                                    local,
                                    stroke.color,
                                )?
                                .0,
                            )),
                        };
                        let shadow = p.shadow.as_ref().map(|shadow| {
                            (
                                shadow.offset_x * scale,
                                shadow.offset_y * scale,
                                shadow.blur * scale,
                                shadow.color.0,
                            )
                        });
                        let glow = p.glow.then(|| {
                            (
                                (p.font_size * scale * GLOW_SIZE).clamp(0.0, GLOW_MAX),
                                fill.0,
                            )
                        });
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
                        let face = TextFace {
                            family: p.font.as_str(),
                            style: &style,
                            letter_spacing: p.letter_spacing,
                            line_height: p.line_height,
                            align: p.align,
                            decoration: p.decoration,
                            outline,
                            shadow,
                            glow,
                        };
                        // The raster grows by every decoration's margin on each side; the text keeps its place.
                        let margin = f64::from(face.decoration_margin(background));
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
                            face,
                            background,
                            None,
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
                            let _word_work = if p.word_highlight {
                                Some(
                                    hooks.reserve(
                                        MemoryKind::Compositor,
                                        128 + cue
                                            .texts
                                            .values()
                                            .map(|value| value.text.as_str().len() as u64 * 32)
                                            .sum::<u64>()
                                            + match &cue.word_timing {
                                                captions::CueWordTiming::Words {
                                                    records, ..
                                                } => records.len() as u64 * 64,
                                                _ => 0,
                                            },
                                    )?,
                                )
                            } else {
                                None
                            };
                            let (value, _joined) = super::caption::display_text(
                                cue,
                                &self.settings.caption_languages,
                                hooks,
                            )?;
                            if !value.is_empty() {
                                let selected = super::caption::item_style(self.project.data(), p);
                                let cue_style = p.cues.style_of(cue);
                                let placement_style =
                                    if selected.is_some() { None } else { cue_style };
                                let style = selected.unwrap_or_else(|| {
                                    super::caption::for_cue(&self.settings.caption_style, cue_style)
                                });
                                let caption_placement =
                                    super::caption::cue_placement(&style, placement_style);
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
                                        align: super::caption::align(placement_style),
                                        decoration: super::caption::decoration(placement_style),
                                        ..Default::default()
                                    },
                                    background,
                                    Some((
                                        &super::caption::word_plan(
                                            p,
                                            cue,
                                            &self.settings.caption_languages,
                                            p.cues.cues.iter().any(|other| {
                                                !std::ptr::eq(other, cue)
                                                    && other.range.start() < cue.range.end()
                                                    && cue.range.start() < other.range.end()
                                            }),
                                        ),
                                        source_at,
                                    )),
                                    hooks,
                                )?;
                                draw(&mut out, &image, caption_placement, hooks)?;
                            }
                        }
                    }
                    ItemParams::Audio(_) => {}
                }
                if let Some(mut below) = below {
                    let mut filtered = out;
                    for run in &runs {
                        if run.is_empty() {
                            continue;
                        }
                        filtered =
                            crate::effects::apply_effect(filtered, run, &below, adapter, hooks)?;
                    }
                    if filtered.width != below.width || filtered.height != below.height {
                        return Err(Error::Invalid("effect output dimensions"));
                    }
                    if masked {
                        adapter.release_idle()?;
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
                                    None,
                                    hooks,
                                )
                            },
                            hooks,
                        )?;
                    }

                    if transform.active() {
                        adapter.release_idle()?;
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
                        Err(Error::Unsupported(_)) => {
                            adapter.release_idle()?;
                            crate::blend::composite(
                                &mut below,
                                &filtered,
                                final_opacity,
                                item.blend_mode,
                                hooks,
                            )?;
                        }
                        result => result?,
                    }
                    out = below;
                }
            }
            if let Some((tr, is_outgoing)) = transition {
                adapter.release_idle()?;
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
        word_plan: Option<(&super::caption::WordPlan, time::Tick)>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Arc<Image>> {
        let key_bytes = (value.len()
            + face.family.len()
            + caption.map_or(face.style, |s| s.font_style.as_str()).len())
            as u64
            + 512
            + word_plan.map_or(0, |(plan, _)| plan.spans.len() as u64 * 32);
        let key_lease = match hooks.reserve(MemoryKind::Compositor, key_bytes) {
            Err(Error::Budget { .. }) if !self.text_cache.is_empty() => {
                self.clear_text_cache();
                hooks.reserve(MemoryKind::Compositor, key_bytes)?
            }
            result => result?,
        };
        let key = self.text_key(
            value, size, fill, placement, caption, face, background, word_plan,
        )?;
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
        let bytes = Image::bytes(key.width, key.height)?
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
    #[allow(clippy::too_many_arguments)]
    fn text_key(
        &self,
        value: &str,
        size: f32,
        fill: [u8; 4],
        placement: Placement,
        caption: Option<&CaptionStyle>,
        face: TextFace<'_>,
        background: Option<TextBox>,
        word_plan: Option<(&super::caption::WordPlan, time::Tick)>,
    ) -> Result<TextKey> {
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
        let highlight = caption.map_or([0; 3], |style| style.word_highlight_color);
        Ok(TextKey {
            value: value.to_owned(),
            highlight_rgb: highlight,
            word_spans: word_plan.map_or_else(Vec::new, |(plan, _)| {
                plan.spans
                    .iter()
                    .filter(|(range, _, _)| range.is_some())
                    .map(|&(_, start, end)| (start, end))
                    .collect()
            }),
            emphasis: word_plan.map_or_else(Vec::new, |(plan, _)| plan.emphasis.clone()),
            active_word: word_plan.and_then(|(plan, at)| {
                plan.spans
                    .iter()
                    .find(|(range, _, _)| {
                        range.is_some_and(|range| range.start() <= at && at < range.end())
                    })
                    .map(|&(_, start, end)| (start, end, highlight))
            }),
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
                .map(|o| ((o.width * scale).to_bits(), o.color))
                .or_else(|| face.outline.map(|(width, color)| (width.to_bits(), color))),
            shadow: face
                .shadow
                .map(|(x, y, blur, color)| (x.to_bits(), y.to_bits(), blur.to_bits(), color)),
            glow: face.glow.map(|(radius, color)| (radius.to_bits(), color)),
            background: background.map(TextBox::key),
            align: face.align,
            decoration: face.decoration,
            position: caption.map_or(CaptionPosition::Top, |s| s.position),
            max_lines: caption.map(|s| s.max_lines),
        })
    }

    /// Recompute bounded safe warnings for this immutable project and display settings.
    /// Does not run jobs, decode media or mutate history. Call on committed-state changes.
    pub fn caption_word_warnings(
        &self,
        target: Option<project::ItemId>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<project::CaptionWordWarnings> {
        use captions::WordHighlightReason;
        let mut result = project::CaptionWordWarnings::default();
        for track in self
            .project
            .data()
            .scene()
            .timeline
            .iter()
            .filter(|track| !track.hidden)
        {
            for item in track.items.iter().filter(|item| !item.hidden) {
                let ItemParams::Caption(params) = &item.content else {
                    continue;
                };
                if !params.word_highlight || target.is_some_and(|target| target != item.id) {
                    continue;
                }
                let _intervals =
                    hooks.reserve(MemoryKind::Compositor, params.cues.cues.len() as u64 * 40)?;
                let ambiguous = super::caption::ambiguous_cues(params);
                for (index, cue) in params.cues.cues.iter().enumerate() {
                    check(hooks)?;
                    let _work = hooks.reserve(
                        MemoryKind::Compositor,
                        65536
                            + cue
                                .texts
                                .values()
                                .map(|text| text.text.as_str().len() as u64)
                                .sum::<u64>()
                                * 512
                            + match &cue.word_timing {
                                captions::CueWordTiming::Words { records, .. } => {
                                    records.len() as u64 * 64
                                }
                                _ => 0,
                            },
                    )?;
                    let plan = super::caption::word_plan(
                        params,
                        cue,
                        &self.settings.caption_languages,
                        ambiguous.get(index).copied().unwrap_or(true),
                    );
                    let mut reason = plan.reason;
                    if reason.is_none() {
                        let (value, _joined) = super::caption::display_text(
                            cue,
                            &self.settings.caption_languages,
                            hooks,
                        )?;
                        let selected = super::caption::item_style(self.project.data(), params);
                        let cue_style = params.cues.style_of(cue);
                        let placement_style = if selected.is_some() { None } else { cue_style };
                        let style = selected.unwrap_or_else(|| {
                            super::caption::for_cue(&self.settings.caption_style, cue_style)
                        });
                        let scale = self.settings.height as f32 / 1080.0;
                        let key = self.text_key(
                            &value,
                            style.size,
                            style.color,
                            super::caption::cue_placement(&style, placement_style),
                            Some(&style),
                            TextFace {
                                family: &style.font,
                                style: &style.font_style,
                                align: super::caption::align(placement_style),
                                decoration: super::caption::decoration(placement_style),
                                ..Default::default()
                            },
                            style.background.as_ref().map(|b| {
                                TextBox::uniform(b.padding * scale, b.radius * scale, b.color)
                            }),
                            Some((&plan, time::Tick::ZERO)),
                        )?;
                        let layout = super::raster::fitted_layout(&key, hooks)?;
                        if !layout.certifies_spans(&key.word_spans) {
                            reason = Some(WordHighlightReason::ShapedBoundary);
                        }
                    }
                    if let Some(reason) = reason {
                        let language = match &cue.word_timing {
                            captions::CueWordTiming::Words { language, .. } => language,
                            _ => &cue.source_lang,
                        };
                        if result.warnings.iter().any(|warning| {
                            warning.item == item.id
                                && warning.language == *language
                                && warning.reason == reason
                        }) {
                            continue;
                        }
                        if result.warnings.len() < 64 {
                            result.warnings.push(project::CaptionWordWarning {
                                item: item.id,
                                cue: index,
                                language: language.clone(),
                                reason,
                            });
                        } else {
                            result.additional = result.additional.saturating_add(1);
                        }
                    }
                }
            }
        }
        Ok(result)
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
/// Paints a clip background over the whole canvas. A blur reads only the clip's
/// own decoded frame, so it never samples the canvas it is painting onto.
fn paint_clip_background(
    out: &mut Image,
    background: project::ClipBackground,
    frame: Option<&Image>,
    item: &Item,
    local: Tick,
    hooks: &dyn ExecutionHooks,
) -> Result<()> {
    match background {
        project::ClipBackground::None => {}
        project::ClipBackground::Solid { color: fill } => {
            let solid = Image::new(out.width, out.height, color(fill.0), hooks)?;
            draw(out, &solid, Placement::default(), hooks)?;
        }
        project::ClipBackground::Blur { intensity } => {
            let Some(frame) = frame else {
                return Ok(());
            };
            let mut fitted = Image::new(out.width, out.height, [0.0; 4], hooks)?;
            draw(
                &mut fitted,
                frame,
                item_crop(item, Placement::default(), local)?,
                hooks,
            )?;
            let blurred = super::effects::clip_background_blur(&fitted, intensity, hooks)?;
            draw(out, &blurred, Placement::default(), hooks)?;
        }
    }
    Ok(())
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
/// Item-local time at which the item's content is sampled: the enabled effects'
/// content-time remaps in stack order, clamped into the item's range by the remap.
/// Only the content is held; transform, crop, opacity and effect parameters keep the
/// real time. Without a remap the time is the item's own, so transition handles
/// outside the item's extent sample exactly as before.
fn content_local(item: &Item, local: Tick, width: u32, height: u32) -> Result<Tick> {
    crate::effects::content_time(item, local, width, height)
}
/// The timeline tick of an item-local content time.
fn content_tick(item: &Item, local: Tick) -> Result<Tick> {
    item.start
        .checked_add(local)
        .ok_or(Error::Invalid("item content time"))
}
fn local_time(item: &Item, at: Tick) -> Result<Tick> {
    at.checked_sub(item.start)
        .ok_or(Error::Invalid("item source mapping"))
}
