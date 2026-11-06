//! Rasters and caches text for the frame.

use crate::{CaptionPosition, Error, ExecutionHooks, Image, MemoryKind, Result, check};
use std::sync::Arc;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct TextKey {
    pub(super) value: String,
    pub(super) size: u32,
    pub(super) fill: [u8; 4],
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) font: String,
    pub(super) style: String,
    pub(super) letter_spacing: u32,
    pub(super) line_height: u32,
    /// Registered-font generation: a newly loaded family changes what a name resolves to.
    pub(super) fonts: usize,
    pub(super) outline: Option<(u32, [u8; 4])>,
    pub(super) background: Option<(u32, u32, [u8; 4])>,
    pub(super) position: CaptionPosition,
    pub(super) max_lines: Option<usize>,
}
/// Family, style and spacing that select and space a text raster.
#[derive(Clone, Copy)]
pub(super) struct TextFace<'a> {
    pub(super) family: &'a str,
    pub(super) style: &'a str,
    /// Project canvas pixels (caption pixels for captions), scaled with the font size.
    pub(super) letter_spacing: f32,
    pub(super) line_height: f32,
}
impl Default for TextFace<'_> {
    fn default() -> Self {
        Self {
            family: "Inter",
            style: "Regular",
            letter_spacing: 0.0,
            line_height: 0.0,
        }
    }
}
pub(super) struct TextEntry {
    pub(super) image: Arc<Image>,
    pub(super) stamp: u64,
    pub(super) bytes: u64,
    pub(super) _key: Box<dyn crate::Reservation>,
}

pub(super) fn raster_text(key: &TextKey, hooks: &dyn ExecutionHooks) -> Result<Image> {
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
        letter_spacing: f32::from_bits(key.letter_spacing),
        line_height: f32::from_bits(key.line_height),
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
