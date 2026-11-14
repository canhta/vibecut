//! The cutout stroke: an outline drawn around the person cut out by background removal. It runs
//! on the item's layer right after the matte, as a CPU pass; the picture's alpha is the shape, so
//! the outline follows the matte edge, and it sits under the person and over whatever the layer
//! still holds (a caption behind the clip is covered, as in the reference).
use crate::{Error, ExecutionHooks, Image, Result, check};
use project::{CutoutStroke, StrokePreset};

/// Outline radius at Size 0 and Size 100 as a fraction of the layer's shorter side. Measured
/// on capture round 4 (frames 151613, 151605, 151616, 3680 px frames, player 1340 x 754 px): the
/// outline is thin (about one pixel) at Size 0 and about 13 and 40 px thick across a steep edge
/// at 35 and 100, linear in Size to within the reading error; 40 / 754 is the 0.053.
const THIN: f32 = 0.0013;
const WIDE: f32 = 0.053;
/// Alpha at or above which a layer pixel counts as the person.
const SHAPE: f32 = 0.5;
/// Stands for no shape pixel; finite so the squared-distance arithmetic stays exact enough.
const FAR: f64 = 1e15;

/// Outline radius in layer pixels; never under one pixel, so Size 0 stays a thin line.
pub fn radius(size: f32, width: u32, height: u32) -> f32 {
    ((THIN + size.clamp(0.0, 100.0) / 100.0 * (WIDE - THIN)) * width.min(height) as f32).max(1.0)
}

/// One 1-D squared-distance transform (Felzenszwalb and Huttenlocher): `out[q]` is the lowest
/// `(q - p)^2 + f[p]`. `hull` and `bounds` are scratch the caller reuses.
fn transform(f: &[f64], out: &mut Vec<f64>, hull: &mut Vec<usize>, bounds: &mut Vec<f64>) {
    let at = |i: usize| f.get(i).copied().unwrap_or(0.0);
    out.clear();
    hull.clear();
    bounds.clear();
    if f.is_empty() {
        return;
    }
    hull.push(0);
    bounds.extend([f64::NEG_INFINITY, f64::INFINITY]);
    for q in 1..f.len() {
        loop {
            let p = hull.last().copied().unwrap_or(0);
            let s = ((at(q) + (q * q) as f64) - (at(p) + (p * p) as f64))
                / (2.0 * (q as f64 - p as f64));
            let lower = bounds
                .len()
                .checked_sub(2)
                .and_then(|i| bounds.get(i))
                .copied()
                .unwrap_or(f64::NEG_INFINITY);
            if s <= lower && hull.len() > 1 {
                hull.pop();
                bounds.pop();
                continue;
            }
            bounds.pop();
            bounds.extend([s, f64::INFINITY]);
            hull.push(q);
            break;
        }
    }
    let mut k = 0usize;
    for q in 0..f.len() {
        while bounds.get(k + 1).is_some_and(|edge| *edge < q as f64) {
            k += 1;
        }
        let p = hull.get(k).copied().unwrap_or(0);
        let gap = q as f64 - p as f64;
        out.push(gap * gap + at(p));
    }
}

/// Draw the stroke around the opaque part of `layer`. A disabled stroke, the None preset, an
/// opacity of 0 or a layer with no person leaves the layer as it is.
pub fn apply(mut layer: Image, stroke: &CutoutStroke, hooks: &dyn ExecutionHooks) -> Result<Image> {
    check(hooks)?;
    if !stroke.enabled || stroke.preset == StrokePreset::None || stroke.opacity <= 0.0 {
        return Ok(layer);
    }
    let (width, height) = (layer.width() as usize, layer.height() as usize);
    if width == 0 || height == 0 || layer.pixels.len() != width * height {
        return Err(Error::Invalid("cutout stroke layer"));
    }
    let mut field: Vec<f64> = layer
        .pixels
        .iter()
        .map(|pixel| if pixel[3] >= SHAPE { 0.0 } else { FAR })
        .collect();
    if field.iter().all(|value| *value >= FAR) {
        return Ok(layer);
    }
    let (mut line, mut out) = (Vec::new(), Vec::new());
    let (mut hull, mut bounds) = (Vec::new(), Vec::new());
    for x in 0..width {
        check(hooks)?;
        line.clear();
        line.extend(field.iter().skip(x).step_by(width).copied());
        transform(&line, &mut out, &mut hull, &mut bounds);
        for (slot, value) in field.iter_mut().skip(x).step_by(width).zip(&out) {
            *slot = *value;
        }
    }
    for row in field.chunks_exact_mut(width) {
        check(hooks)?;
        line.clear();
        line.extend_from_slice(row);
        transform(&line, &mut out, &mut hull, &mut bounds);
        row.copy_from_slice(&out);
    }
    let reach = f64::from(radius(stroke.size, layer.width(), layer.height()));
    let [red, green, blue, _] = stroke.color.0;
    let color = [red, green, blue].map(|v| crate::image::eotf(f32::from(v) / 255.0, 13));
    let opacity = stroke.opacity.clamp(0.0, 100.0) / 100.0;
    for (pixel, squared) in layer.pixels.iter_mut().zip(&field) {
        let coverage = (reach + 0.5 - squared.sqrt()).clamp(0.0, 1.0) as f32;
        let alpha = coverage * opacity;
        let room = 1.0 - pixel[3];
        if alpha <= 0.0 || room <= 0.0 {
            continue;
        }
        for (channel, tint) in pixel.iter_mut().zip(color) {
            *channel += room * tint * alpha;
        }
        pixel[3] += room * alpha;
    }
    Ok(layer)
}
