//! Rasters and caches text for the frame.

use crate::{CaptionPosition, Error, ExecutionHooks, Image, MemoryKind, Result, check};
use project::{TextAlign, TextDecoration};
use std::sync::Arc;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct TextKey {
    pub(super) value: String,
    pub(super) word_spans: Vec<(usize, usize)>,
    pub(super) active_word: Option<(usize, usize, [u8; 3])>,
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
    pub(super) background: Option<BoxKey>,
    pub(super) align: TextAlign,
    pub(super) decoration: TextDecoration,
    pub(super) position: CaptionPosition,
    pub(super) max_lines: Option<usize>,
}
/// Background box in output pixels (float bits so the key stays hashable and ordered).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct BoxKey {
    pub(super) padding_x: u32,
    pub(super) padding_y: u32,
    pub(super) offset_x: u32,
    pub(super) offset_y: u32,
    pub(super) radius: u32,
    pub(super) color: [u8; 4],
}
/// Background box in output pixels.
#[derive(Clone, Copy)]
pub(super) struct TextBox {
    pub(super) padding_x: f32,
    pub(super) padding_y: f32,
    pub(super) offset_x: f32,
    pub(super) offset_y: f32,
    pub(super) radius: f32,
    pub(super) color: [u8; 4],
}
impl BoxKey {
    /// Pixels the raster grows beyond the text area on every side so the padded, shifted box
    /// is never clipped and the text itself never moves with the box.
    pub(super) fn margin(self) -> f32 {
        let (px, py) = (
            f32::from_bits(self.padding_x),
            f32::from_bits(self.padding_y),
        );
        let (ox, oy) = (f32::from_bits(self.offset_x), f32::from_bits(self.offset_y));
        (px + ox.abs()).max(py + oy.abs()).ceil()
    }
}
impl TextBox {
    pub(super) fn margin(self) -> f32 {
        self.key().margin()
    }
    pub(super) fn uniform(padding: f32, radius: f32, color: [u8; 4]) -> Self {
        Self {
            padding_x: padding,
            padding_y: padding,
            offset_x: 0.0,
            offset_y: 0.0,
            radius,
            color,
        }
    }
    pub(super) fn key(self) -> BoxKey {
        BoxKey {
            padding_x: self.padding_x.to_bits(),
            padding_y: self.padding_y.to_bits(),
            offset_x: self.offset_x.to_bits(),
            offset_y: self.offset_y.to_bits(),
            radius: self.radius.to_bits(),
            color: self.color,
        }
    }
}
/// Family, style and spacing that select and space a text raster.
#[derive(Clone, Copy)]
pub(super) struct TextFace<'a> {
    pub(super) family: &'a str,
    pub(super) style: &'a str,
    /// Project canvas pixels (caption pixels for captions), scaled with the font size.
    pub(super) letter_spacing: f32,
    pub(super) line_height: f32,
    pub(super) align: TextAlign,
    pub(super) decoration: TextDecoration,
}
impl Default for TextFace<'_> {
    fn default() -> Self {
        Self {
            family: "Inter",
            style: "Regular",
            letter_spacing: 0.0,
            line_height: 0.0,
            align: TextAlign::Center,
            decoration: TextDecoration::None,
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
    let background = key.background.map(|b| text::Rectangle {
        padding_x: f32::from_bits(b.padding_x),
        padding_y: f32::from_bits(b.padding_y),
        offset_x: f32::from_bits(b.offset_x),
        offset_y: f32::from_bits(b.offset_y),
        radius: f32::from_bits(b.radius),
        color: rgba(b.color),
    });
    // Margin that keeps outline and the offset box inside the canvas on every side.
    let pad = outline
        .map_or(0.0, |o| o.width)
        .max(key.background.map_or(0.0, BoxKey::margin))
        .ceil();
    if w as f32 <= pad * 2.0 || h as f32 <= pad * 2.0 {
        return Err(Error::Invalid("caption decoration exceeds destination"));
    }
    let _scratch = hooks.reserve(
        MemoryKind::Compositor,
        bytes * (2 + u64::from(key.active_word.is_some()))
            + key.value.len() as u64 * 512
            + (f64::from(size).powi(2) * 32.0).ceil() as u64,
    )?;
    let layout = fitted_layout(key, hooks)?;
    let b = layout.bounds;
    let y = if key.max_lines.is_none() {
        pad
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
            span_fill: key
                .active_word
                .filter(|_| layout.certifies_spans(&key.word_spans))
                .map(|(start, end, rgb)| text::SpanFill { start, end, rgb }),
            outline,
            background,
            decoration: match key.decoration {
                TextDecoration::None => text::Decoration::None,
                TextDecoration::Underline => text::Decoration::Underline,
                TextDecoration::LineThrough => text::Decoration::LineThrough,
            },
        },
    )?;
    Image::from_rgba8(w, h, &image.pixels, true, 13, hooks)
}
fn rgba(value: [u8; 4]) -> text::Color {
    let [r, g, b, a] = value;
    text::Color::rgba(r, g, b, a)
}

pub(super) fn fitted_layout(key: &TextKey, hooks: &dyn ExecutionHooks) -> Result<text::Layout> {
    let (w, h, size) = (key.width, key.height, f32::from_bits(key.size));
    let pad = key
        .outline
        .map_or(0.0, |(width, _)| f32::from_bits(width))
        .max(key.background.map_or(0.0, BoxKey::margin))
        .ceil();
    let mut options = text::LayoutOptions {
        family: &key.font,
        style: &key.style,
        letter_spacing: f32::from_bits(key.letter_spacing),
        line_height: f32::from_bits(key.line_height),
        size,
        max_width: w as f32 - pad * 2.0,
        align: match key.align {
            TextAlign::Left => text::Align::Left,
            TextAlign::Center => text::Align::Center,
            TextAlign::Right => text::Align::Right,
        },
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
    Ok(layout)
}
