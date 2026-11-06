//! Glyph outline extraction and compositing into premultiplied RGBA8 images.

use skrifa::GlyphId;
use skrifa::font::glyph::outline::{DrawMemory, DrawOptions};
use skrifa::outline::OutlinePen;

use crate::color::Color;
use crate::fonts;
use crate::layout::{Glyph, Layout};
use crate::mask::{self, StrokeKind};
use crate::raster::{Mask, Path, Xform, fill_into};

/// Synthetic italic slant (tan 12°).
const SYNTH_ITALIC_SLANT: f32 = 0.2126;
/// Synthetic bold outline growth as a fraction of the font size.
const SYNTH_BOLD: f32 = 0.025;
/// Decoration bar thickness as a fraction of the font size.
const DECORATION_THICKNESS: f32 = 0.07;
/// Underline top below the baseline as a fraction of the font size.
const UNDERLINE_OFFSET: f32 = 0.1;
/// Line-through centre above the baseline as a fraction of the font size.
const STRIKE_HEIGHT: f32 = 0.3;
/// Largest accepted canvas, bounding the RGBA allocation to 256 MiB.
const MAX_CANVAS_PIXELS: u64 = 64 << 20;

/// Output dimensions in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
}

/// An outline drawn around the glyph fill.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Outline {
    pub color: Color,
    /// Stroke width in pixels.
    pub width: f32,
    pub kind: StrokeKind,
}

/// A background box around the layout's content bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rectangle {
    pub color: Color,
    /// Extra space in pixels left and right of the content bounds.
    pub padding_x: f32,
    /// Extra space in pixels above and below the content bounds.
    pub padding_y: f32,
    /// Shift of the whole box from the content bounds, in pixels (y down).
    pub offset_x: f32,
    pub offset_y: f32,
    /// Corner radius in pixels; zero draws square corners.
    pub radius: f32,
}

impl Rectangle {
    /// Box with the same padding on every side and no offset.
    pub fn uniform(color: Color, padding: f32, radius: f32) -> Self {
        Self {
            color,
            padding_x: padding,
            padding_y: padding,
            offset_x: 0.0,
            offset_y: 0.0,
            radius,
        }
    }
}

/// A line drawn across every laid-out line in the fill colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Decoration {
    #[default]
    None,
    Underline,
    LineThrough,
}

/// How a layout is painted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paint {
    pub fill: Color,
    pub outline: Option<Outline>,
    pub background: Option<Rectangle>,
    pub decoration: Decoration,
}

impl Default for Paint {
    fn default() -> Self {
        Self {
            fill: Color::WHITE,
            outline: None,
            background: None,
            decoration: Decoration::None,
        }
    }
}

/// A premultiplied RGBA8 image, row-major, four bytes per pixel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Invalid canvas or paint input.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("text coverage cancelled")]
    Cancelled,
    #[error("coverage must be finite and in 0..=1")]
    InvalidCoverage,
    #[error("text coverage allocation failed")]
    Allocation(#[from] std::collections::TryReserveError),
    #[error("canvas dimensions must be positive and at most {MAX_CANVAS_PIXELS} pixels")]
    InvalidCanvas,
    #[error("outline width must be finite and not negative")]
    InvalidOutline,
    #[error("background padding and radius must be finite and not negative")]
    InvalidRectangle,
}

#[derive(Clone, Copy, Debug)]
enum Cmd {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Curve(f32, f32, f32, f32, f32, f32),
    Close,
}

struct Recorder(Vec<Cmd>);

impl OutlinePen for Recorder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push(Cmd::Move(x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push(Cmd::Line(x, y));
    }

    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.0.push(Cmd::Quad(cx, cy, x, y));
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.push(Cmd::Curve(cx0, cy0, cx1, cy1, x, y));
    }

    fn close(&mut self) {
        self.0.push(Cmd::Close);
    }
}

fn glyph_path(glyph: &Glyph) -> Path {
    let mut path = Path::new();
    let Some(font) = fonts::face(glyph.face).font_ref() else {
        return path;
    };
    let Some(outline) = font.outline_glyphs().get(GlyphId::new(glyph.id)) else {
        return path;
    };
    let mut recorder = Recorder(Vec::new());
    if outline
        .draw(&DrawOptions::new(), DrawMemory::Scratch, &mut recorder)
        .is_err()
    {
        return path;
    }
    let scale = fonts::face(glyph.face).scale(glyph.size);
    let slant = if glyph.synth_italic {
        SYNTH_ITALIC_SLANT
    } else {
        0.0
    };
    let transform = Xform {
        a: scale,
        b: 0.0,
        c: slant * scale,
        d: -scale,
        e: 0.0,
        f: 0.0,
    };
    path.detail = 1.0;
    for command in &recorder.0 {
        match *command {
            Cmd::Move(x, y) => {
                let (x, y) = transform.apply(x, y);
                path.move_to(x, y);
            }
            Cmd::Line(x, y) => {
                let (x, y) = transform.apply(x, y);
                path.line_to(x, y);
            }
            Cmd::Quad(cx, cy, x, y) => {
                let (cx, cy) = transform.apply(cx, cy);
                let (x, y) = transform.apply(x, y);
                path.quad_to(cx, cy, x, y);
            }
            Cmd::Curve(cx0, cy0, cx1, cy1, x, y) => {
                let (cx0, cy0) = transform.apply(cx0, cy0);
                let (cx1, cy1) = transform.apply(cx1, cy1);
                let (x, y) = transform.apply(x, y);
                path.cubic_to(cx0, cy0, cx1, cy1, x, y);
            }
            Cmd::Close => path.close(),
        }
    }
    path.close();
    if glyph.synth_bold {
        path.embolden(glyph.size * SYNTH_BOLD);
    }
    path
}

/// Render a layout into a premultiplied RGBA8 image.
///
/// `origin` maps layout coordinates to canvas pixels. The background box, outline and fill are
/// composited in that order, with an outer outline below the fill and centre/inner outlines above
/// it.
pub fn render(
    layout: &Layout,
    canvas: Canvas,
    origin: (f32, f32),
    paint: &Paint,
) -> Result<Image, RenderError> {
    let pixels = u64::from(canvas.width)
        .checked_mul(u64::from(canvas.height))
        .filter(|pixels| *pixels > 0 && *pixels <= MAX_CANVAS_PIXELS)
        .ok_or(RenderError::InvalidCanvas)?;
    if let Some(outline) = paint.outline
        && !(outline.width.is_finite() && outline.width >= 0.0)
    {
        return Err(RenderError::InvalidOutline);
    }
    if let Some(background) = paint.background
        && !(background.padding_x.is_finite()
            && background.padding_x >= 0.0
            && background.padding_y.is_finite()
            && background.padding_y >= 0.0
            && background.offset_x.is_finite()
            && background.offset_y.is_finite()
            && background.radius.is_finite()
            && background.radius >= 0.0)
    {
        return Err(RenderError::InvalidRectangle);
    }

    let width = canvas.width as usize;
    let height = canvas.height as usize;
    let mut image = Image {
        width: canvas.width,
        height: canvas.height,
        pixels: vec![0u8; usize::try_from(pixels.saturating_mul(4)).unwrap_or(0)],
    };
    let (ox, oy) = origin;

    if let Some(background) = paint.background {
        let bounds = layout.bounds;
        let path = Path::round_rect(
            ox + bounds.x0 - background.padding_x + background.offset_x,
            oy + bounds.y0 - background.padding_y + background.offset_y,
            ox + bounds.x1 + background.padding_x + background.offset_x,
            oy + bounds.y1 + background.padding_y + background.offset_y,
            background.radius,
        );
        let mut mask = Mask::new(width, height);
        fill_into(&path, &mut mask, 0.0, 0.0);
        blend(&mut image, &mask, background.color);
    }

    let mut fill_mask = Mask::new(width, height);
    for glyph in &layout.glyphs {
        let path = glyph_path(glyph);
        fill_into(&path, &mut fill_mask, ox + glyph.x, oy + glyph.y);
    }
    if paint.decoration != Decoration::None {
        for line in &layout.lines {
            let size = layout.glyphs[line.glyphs.clone()]
                .first()
                .map_or(0.0, |glyph| glyph.size);
            if size <= 0.0 || line.width <= 0.0 {
                continue;
            }
            let thickness = (size * DECORATION_THICKNESS).max(1.0);
            let top = match paint.decoration {
                Decoration::Underline => line.baseline + size * UNDERLINE_OFFSET,
                _ => line.baseline - size * STRIKE_HEIGHT - thickness / 2.0,
            };
            let bar = Path::round_rect(
                ox + line.x,
                oy + top,
                ox + line.x + line.width,
                oy + top + thickness,
                0.0,
            );
            fill_into(&bar, &mut fill_mask, 0.0, 0.0);
        }
    }

    if let Some(outline) = paint.outline {
        let stroke = mask::stroke(&fill_mask, outline.width, outline.kind)?;
        if outline.kind == StrokeKind::Outer {
            blend(&mut image, &stroke, outline.color);
            blend(&mut image, &fill_mask, paint.fill);
        } else {
            blend(&mut image, &fill_mask, paint.fill);
            blend(&mut image, &stroke, outline.color);
        }
    } else {
        blend(&mut image, &fill_mask, paint.fill);
    }
    Ok(image)
}

fn blend(image: &mut Image, mask: &Mask, color: Color) {
    if color.is_transparent() {
        return;
    }
    let alpha = f32::from(color.a) / 255.0;
    for (index, coverage) in mask.a.iter().enumerate() {
        let local_alpha = alpha * coverage.clamp(0.0, 1.0);
        if local_alpha <= 0.0 {
            continue;
        }
        let Some(pixel) = index
            .checked_mul(4)
            .and_then(|offset| image.pixels.get_mut(offset..offset.saturating_add(4)))
        else {
            continue;
        };
        let inv = 1.0 - local_alpha;
        if let Some(channel) = pixel.get_mut(0) {
            *channel = premultiply(color.r, local_alpha, *channel, inv);
        }
        if let Some(channel) = pixel.get_mut(1) {
            *channel = premultiply(color.g, local_alpha, *channel, inv);
        }
        if let Some(channel) = pixel.get_mut(2) {
            *channel = premultiply(color.b, local_alpha, *channel, inv);
        }
        if let Some(channel) = pixel.get_mut(3) {
            *channel = premultiply(255, local_alpha, *channel, inv);
        }
    }
}

fn premultiply(source: u8, alpha: f32, destination: u8, inverse: f32) -> u8 {
    let source = f32::from(source) * alpha;
    let destination = f32::from(destination) * inverse;
    (source + destination).round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{LayoutOptions, layout};

    #[test]
    fn render_is_premultiplied_and_uses_alpha() {
        let text = "H";
        let laid_out = layout(
            text,
            &LayoutOptions {
                size: 80.0,
                max_width: 400.0,
                ..LayoutOptions::default()
            },
        )
        .expect("layout");
        let image = render(
            &laid_out,
            Canvas {
                width: 320,
                height: 200,
            },
            (20.0, 40.0),
            &Paint {
                fill: Color::rgb(255, 255, 255),
                outline: Some(Outline {
                    color: Color::rgb(0, 0, 0),
                    width: 4.0,
                    kind: StrokeKind::Outer,
                }),
                background: Some(Rectangle::uniform(Color::rgba(0, 0, 0, 160), 12.0, 6.0)),
                decoration: Decoration::None,
            },
        )
        .expect("render");
        assert_eq!(image.pixels.len(), 320 * 200 * 4);
        let mut opaque = 0usize;
        for pixel in image.pixels.as_chunks::<4>().0 {
            assert!(pixel[0] <= pixel[3] && pixel[1] <= pixel[3] && pixel[2] <= pixel[3]);
            if pixel[3] > 0 {
                opaque += 1;
            }
        }
        assert!(opaque > 1000, "{opaque}");
    }

    #[test]
    fn outer_outline_adds_coverage_outside_the_fill() {
        let laid_out = layout(
            "H",
            &LayoutOptions {
                size: 80.0,
                max_width: 400.0,
                ..LayoutOptions::default()
            },
        )
        .expect("layout");
        let canvas = Canvas {
            width: 240,
            height: 160,
        };
        let fill = render(
            &laid_out,
            canvas,
            (20.0, 30.0),
            &Paint {
                fill: Color::WHITE,
                outline: None,
                background: None,
                decoration: Decoration::None,
            },
        )
        .expect("fill");
        let outlined = render(
            &laid_out,
            canvas,
            (20.0, 30.0),
            &Paint {
                fill: Color::WHITE,
                outline: Some(Outline {
                    color: Color::BLACK,
                    width: 8.0,
                    kind: StrokeKind::Outer,
                }),
                background: None,
                decoration: Decoration::None,
            },
        )
        .expect("outline");
        let mut outline_only = 0usize;
        let mut filled = 0usize;
        for (fill_pixel, outline_pixel) in fill
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .zip(outlined.pixels.as_chunks::<4>().0)
        {
            if fill_pixel[3] > 0 {
                filled += 1;
            }
            if fill_pixel[3] == 0 && outline_pixel[3] > 0 {
                outline_only += 1;
            }
        }
        assert!(filled > 100, "{filled}");
        assert!(outline_only > 100, "{outline_only}");
    }

    #[test]
    fn rejects_invalid_canvas_and_outline() {
        let laid_out = layout("x", &LayoutOptions::default()).expect("layout");
        assert!(matches!(
            render(
                &laid_out,
                Canvas {
                    width: 0,
                    height: 10
                },
                (0.0, 0.0),
                &Paint::default()
            ),
            Err(RenderError::InvalidCanvas)
        ));
        assert!(matches!(
            render(
                &laid_out,
                Canvas {
                    width: 10,
                    height: 10
                },
                (0.0, 0.0),
                &Paint {
                    outline: Some(Outline {
                        color: Color::BLACK,
                        width: -1.0,
                        kind: StrokeKind::Outer,
                    }),
                    ..Paint::default()
                }
            ),
            Err(RenderError::InvalidOutline)
        ));
    }
}
