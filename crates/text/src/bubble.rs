//! Draws the own-authored bubble shapes behind a text layout.

use crate::raster::Path;

/// A bubble shape; the caller maps its own vocabulary onto these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BubbleShape {
    Round,
    Pill,
    Oval,
    SpeechDown,
    SpeechLeft,
    Banner,
    Tag,
    Burst,
    Cloud,
    Ticket,
    Frame,
}

/// A bubble behind the layout, sized in units of the font size so it scales with the text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bubble {
    pub shape: BubbleShape,
    pub color: crate::Color,
    /// The font size in pixels: padding, tails and notches are fractions of it.
    pub unit: f32,
}

/// Pixels per unit the bubble can extend beyond the text bounds; a raster grown by this on
/// every side never clips a bubble.
pub const BUBBLE_MARGIN: f32 = 1.1;

/// Closed polygon wound like [`Path::rect`] (positive area, y down) whatever the input order.
fn polygon(path: &mut Path, points: &[(f32, f32)]) {
    let area: f32 = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| a.0 * b.1 - b.0 * a.1)
        .sum();
    let mut ordered = points.to_vec();
    if area < 0.0 {
        ordered.reverse();
    }
    let Some((first, rest)) = ordered.split_first() else {
        return;
    };
    path.move_to(first.0, first.1);
    for point in rest {
        path.line_to(point.0, point.1);
    }
    path.close();
}

fn append(path: &mut Path, other: Path) {
    path.contours.extend(other.contours);
}

fn ellipse(path: &mut Path, cx: f32, cy: f32, rx: f32, ry: f32) {
    let (kx, ky) = (0.552_284_8 * rx, 0.552_284_8 * ry);
    path.move_to(cx + rx, cy);
    path.cubic_to(cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry);
    path.cubic_to(cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy);
    path.cubic_to(cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry);
    path.cubic_to(cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy);
    path.close();
}

/// The outline of `bubble` around content bounds `(x0, y0, x1, y1)` in canvas pixels.
pub(crate) fn path(bubble: &Bubble, x0: f32, y0: f32, x1: f32, y1: f32) -> Path {
    let u = bubble.unit.max(1.0);
    let (l, t, r, b) = (x0 - 0.5 * u, y0 - 0.3 * u, x1 + 0.5 * u, y1 + 0.3 * u);
    let (cx, cy) = ((l + r) / 2.0, (t + b) / 2.0);
    let mut path = Path::new();
    match bubble.shape {
        BubbleShape::Round => append(&mut path, Path::round_rect(l, t, r, b, 0.3 * u)),
        BubbleShape::Pill => append(&mut path, Path::round_rect(l, t, r, b, (b - t) / 2.0)),
        BubbleShape::Oval => ellipse(
            &mut path,
            cx,
            cy,
            (r - l) / 2.0 + 0.5 * u,
            (b - t) / 2.0 + 0.3 * u,
        ),
        BubbleShape::SpeechDown => {
            append(&mut path, Path::round_rect(l, t, r, b, 0.3 * u));
            let x = l + (r - l) * 0.3;
            polygon(
                &mut path,
                &[
                    (x, b - 1.0),
                    (x + 0.45 * u, b - 1.0),
                    (x - 0.1 * u, b + 0.6 * u),
                ],
            );
        }
        BubbleShape::SpeechLeft => {
            append(&mut path, Path::round_rect(l, t, r, b, 0.3 * u));
            polygon(
                &mut path,
                &[
                    (l + 1.0, cy - 0.3 * u),
                    (l + 1.0, cy + 0.2 * u),
                    (l - 0.6 * u, cy + 0.35 * u),
                ],
            );
        }
        BubbleShape::Banner => {
            append(&mut path, Path::rect(l, t, r, b));
            let notch = 0.2 * u;
            polygon(
                &mut path,
                &[
                    (l + 1.0, t),
                    (l - 0.5 * u, t),
                    (l - 0.5 * u + notch, cy),
                    (l - 0.5 * u, b),
                    (l + 1.0, b),
                ],
            );
            polygon(
                &mut path,
                &[
                    (r - 1.0, t),
                    (r + 0.5 * u, t),
                    (r + 0.5 * u - notch, cy),
                    (r + 0.5 * u, b),
                    (r - 1.0, b),
                ],
            );
        }
        BubbleShape::Tag => polygon(
            &mut path,
            &[(l, t), (r, t), (r + 0.55 * u, cy), (r, b), (l, b)],
        ),
        BubbleShape::Burst => {
            let (depth, step) = (0.3 * u, 0.6 * u);
            let mut points = Vec::new();
            let across = ((r - l) / step).round().max(1.0) as usize;
            let down = ((b - t) / step).round().max(1.0) as usize;
            for i in 0..across {
                let x = l + (r - l) * i as f32 / across as f32;
                points.push((x, t));
                points.push((x + (r - l) / across as f32 / 2.0, t - depth));
            }
            for i in 0..down {
                let y = t + (b - t) * i as f32 / down as f32;
                points.push((r, y));
                points.push((r + depth, y + (b - t) / down as f32 / 2.0));
            }
            for i in 0..across {
                let x = r - (r - l) * i as f32 / across as f32;
                points.push((x, b));
                points.push((x - (r - l) / across as f32 / 2.0, b + depth));
            }
            for i in 0..down {
                let y = b - (b - t) * i as f32 / down as f32;
                points.push((l, y));
                points.push((l - depth, y - (b - t) / down as f32 / 2.0));
            }
            polygon(&mut path, &points);
        }
        BubbleShape::Cloud => {
            append(&mut path, Path::round_rect(l, t, r, b, 0.3 * u));
            let radius = 0.3 * u;
            let across = ((r - l) / (0.9 * u)).round().max(1.0) as usize;
            let down = ((b - t) / (0.9 * u)).round().max(1.0) as usize;
            for i in 0..=across {
                let x = l + (r - l) * i as f32 / across as f32;
                ellipse(&mut path, x, t, radius, radius);
                ellipse(&mut path, x, b, radius, radius);
            }
            for i in 1..down {
                let y = t + (b - t) * i as f32 / down as f32;
                ellipse(&mut path, l, y, radius, radius);
                ellipse(&mut path, r, y, radius, radius);
            }
        }
        BubbleShape::Ticket => {
            let c = 0.3 * u;
            polygon(
                &mut path,
                &[
                    (l + c, t),
                    (r - c, t),
                    (r, t + c),
                    (r, b - c),
                    (r - c, b),
                    (l + c, b),
                    (l, b - c),
                    (l, t + c),
                ],
            );
        }
        BubbleShape::Frame => {
            let g = 0.15 * u;
            append(
                &mut path,
                Path::round_rect(l - g, t - g, r + g, b + g, 0.2 * u),
            );
            // The inner contour winds the other way, so the non-zero fill leaves a hole.
            path.move_to(l, t);
            path.line_to(l, b);
            path.line_to(r, b);
            path.line_to(r, t);
            path.close();
        }
    }
    path
}
