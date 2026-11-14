//! Resolves a text effect into paint values and picks a bubble's colour.

use captions::{CaptionStyle, TextEffect, outline_width};

/// Fill, outline, shadow and glow of one effect in output pixels.
pub(in crate::scene) struct Recipe {
    pub(in crate::scene) fill: [u8; 4],
    pub(in crate::scene) outline: Option<(f32, [u8; 4])>,
    /// Offset x, offset y, blur and colour.
    pub(in crate::scene) shadow: Option<(f32, f32, f32, [u8; 4])>,
    pub(in crate::scene) glow: Option<(f32, [u8; 4])>,
}

const fn rgb(c: [u8; 3]) -> [u8; 4] {
    [c[0], c[1], c[2], 255]
}

/// The effect's paint at font size `size` pixels; the fill keeps `alpha`.
pub(in crate::scene) fn recipe(effect: TextEffect, size: f32, alpha: u8) -> Recipe {
    let s = size;
    let mut r = Recipe {
        fill: [255, 255, 255, 255],
        outline: None,
        shadow: None,
        glow: None,
    };
    match effect {
        TextEffect::Neon => {
            r.glow = Some((0.35 * s, rgb([0, 230, 255])));
            r.outline = Some((0.03 * s, rgb([0, 230, 255])));
        }
        TextEffect::Pop => {
            r.fill = rgb([255, 214, 0]);
            r.outline = Some((0.12 * s, rgb([0, 0, 0])));
            r.shadow = Some((0.06 * s, 0.06 * s, 0.0, rgb([0, 0, 0])));
        }
        TextEffect::Comic => {
            r.outline = Some((0.14 * s, rgb([0, 0, 0])));
            r.shadow = Some((0.08 * s, 0.08 * s, 0.0, rgb([255, 60, 60])));
        }
        TextEffect::Hollow => {
            r.fill = [0, 0, 0, 0];
            r.outline = Some((0.05 * s, rgb([255, 255, 255])));
        }
        TextEffect::Retro => {
            r.fill = rgb([255, 120, 170]);
            r.outline = Some((0.08 * s, rgb([60, 20, 120])));
            r.shadow = Some((0.1 * s, 0.1 * s, 0.0, rgb([60, 20, 120])));
        }
        TextEffect::Sticker => {
            r.fill = rgb([30, 30, 30]);
            r.outline = Some((0.2 * s, rgb([255, 255, 255])));
            r.shadow = Some((0.0, 0.05 * s, 0.06 * s, [0, 0, 0, 150]));
        }
        TextEffect::Fire => {
            r.fill = rgb([255, 235, 120]);
            r.glow = Some((0.3 * s, rgb([255, 90, 0])));
            r.shadow = Some((0.0, 0.04 * s, 0.05 * s, [120, 0, 0, 200]));
        }
        TextEffect::Ice => {
            r.fill = rgb([225, 245, 255]);
            r.outline = Some((0.08 * s, rgb([30, 110, 200])));
            r.glow = Some((0.25 * s, rgb([90, 180, 255])));
        }
        TextEffect::LongShadow => {
            r.shadow = Some((0.14 * s, 0.14 * s, 0.0, [0, 0, 0, 200]));
        }
        TextEffect::Gold => {
            r.fill = rgb([255, 200, 60]);
            r.outline = Some((0.07 * s, rgb([120, 70, 0])));
            r.shadow = Some((0.03 * s, 0.05 * s, 0.04 * s, [0, 0, 0, 160]));
        }
        TextEffect::Soft => {
            r.shadow = Some((0.0, 0.04 * s, 0.12 * s, [0, 0, 0, 160]));
        }
        TextEffect::Glitch => {
            r.fill = rgb([0, 255, 200]);
            r.shadow = Some((0.06 * s, 0.0, 0.0, rgb([255, 0, 90])));
        }
    }
    r.fill[3] = ((u16::from(r.fill[3]) * u16::from(alpha)) / 255) as u8;
    r
}

/// Paints a caption effect into `style` and returns the shadow and glow the style cannot hold.
/// `scale` maps the caption size to output pixels.
#[allow(clippy::type_complexity)]
pub(in crate::scene) fn caption_effect(
    style: &mut CaptionStyle,
    effect: Option<TextEffect>,
    scale: f32,
) -> (Option<(f32, f32, f32, [u8; 4])>, Option<(f32, [u8; 4])>) {
    let Some(effect) = effect else {
        return (None, None);
    };
    let size = style.size * scale;
    let r = recipe(effect, size, style.color[3]);
    style.color = r.fill;
    style.outline = r.outline.map(|(width, color)| captions::CaptionOutline {
        color,
        // Invert `outline_width` at the style's own size so the key resolves the same pixels.
        thickness: (width / outline_width(1.0, size)).clamp(0.0, captions::MAX_THICKNESS),
    });
    (r.shadow, r.glow)
}

/// A square premultiplied RGBA8 sample of the word `Aa` in the given look, for a picker tile.
/// `None` when the text cannot be laid out.
pub fn sample(
    bubble: Option<captions::TextBubble>,
    effect: Option<TextEffect>,
    side: u32,
) -> Option<Vec<u8>> {
    let side_px = side as f32;
    let size = side_px * if bubble.is_some() { 0.26 } else { 0.4 };
    let layout = text::layout(
        "Aa",
        &text::LayoutOptions {
            style: "Bold",
            size,
            max_width: side_px,
            align: text::Align::Center,
            ..Default::default()
        },
    )
    .ok()?;
    let b = layout.bounds;
    let origin = (
        (side_px - (b.x1 - b.x0)) / 2.0 - b.x0,
        (side_px - (b.y1 - b.y0)) / 2.0 - b.y0,
    );
    // Tiles sit on a dark ground: a light bubble, dark text, except the frame whose hole shows
    // the ground and so keeps light text.
    let mut fill = if bubble.is_some_and(|b| b != captions::TextBubble::Frame) {
        [30, 30, 30, 255]
    } else {
        [255; 4]
    };
    let (mut outline, mut shadow, mut glow) = (None, None, None);
    if let Some(effect) = effect {
        let r = recipe(effect, size, 255);
        (fill, outline, shadow, glow) = (r.fill, r.outline, r.shadow, r.glow);
    }
    let rgba = |c: [u8; 4]| text::Color::rgba(c[0], c[1], c[2], c[3]);
    let image = text::render(
        &layout,
        text::Canvas {
            width: side,
            height: side,
        },
        origin,
        &text::Paint {
            fill: rgba(fill),
            outline: outline.map(|(width, color)| text::Outline {
                width,
                color: rgba(color),
                kind: text::StrokeKind::Outer,
            }),
            shadow: shadow.map(|(x, y, blur, color)| text::Shadow {
                offset_x: x,
                offset_y: y,
                blur,
                color: rgba(color),
            }),
            glow: glow.map(|(radius, color)| text::Glow {
                radius,
                color: rgba(color),
            }),
            bubble: bubble.map(|shape| text::Bubble {
                shape: super::bubble_shape(shape),
                color: rgba([250, 250, 250, 240]),
                unit: size,
            }),
            ..Default::default()
        },
    )
    .ok()?;
    Some(image.pixels)
}

/// A bubble's colour: light behind dark text, dark behind light text.
pub(super) fn bubble_color(fill: [u8; 4]) -> [u8; 4] {
    let luma = 0.299 * f32::from(fill[0]) + 0.587 * f32::from(fill[1]) + 0.114 * f32::from(fill[2]);
    if luma > 140.0 {
        [24, 24, 32, 235]
    } else {
        [250, 250, 250, 240]
    }
}
