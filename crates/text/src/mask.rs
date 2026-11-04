//! Coverage operations for text appearance: outlines from an exact Euclidean distance transform.

use crate::raster::Mask;

/// Where an outline sits relative to the shape's edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StrokeKind {
    /// Grows outwards from the edge and covers the fill.
    #[default]
    Outer,
    /// Straddles the edge.
    Center,
    /// Lies inside the shape.
    Inner,
}

const INF: f32 = 1e20;

/// One-dimensional squared distance transform (Felzenszwalb & Huttenlocher).
fn edt_1d(f: &[f32], d: &mut [f32], v: &mut [usize], z: &mut [f32]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let mut k = 0usize;
    if let Some(slot) = v.get_mut(0) {
        *slot = 0;
    }
    if let Some(slot) = z.get_mut(0) {
        *slot = -INF;
    }
    if let Some(slot) = z.get_mut(1) {
        *slot = INF;
    }
    for q in 1..n {
        let fq = f.get(q).copied().unwrap_or(INF) + (q * q) as f32;
        let mut s;
        loop {
            let p = v.get(k).copied().unwrap_or(0);
            let fp = f.get(p).copied().unwrap_or(INF);
            s = (fq - (fp + (p * p) as f32)) / (2.0 * (q as f32 - p as f32));
            if s <= z.get(k).copied().unwrap_or(-INF) && k > 0 {
                k -= 1;
            } else {
                break;
            }
        }
        if s <= z.get(k).copied().unwrap_or(-INF) {
            if let Some(slot) = v.get_mut(0) {
                *slot = q;
            }
            if let Some(slot) = z.get_mut(0) {
                *slot = -INF;
            }
            if let Some(slot) = z.get_mut(1) {
                *slot = INF;
            }
            continue;
        }
        k += 1;
        if let Some(slot) = v.get_mut(k) {
            *slot = q;
        }
        if let Some(slot) = z.get_mut(k) {
            *slot = s;
        }
        if let Some(slot) = z.get_mut(k + 1) {
            *slot = INF;
        }
    }
    k = 0;
    for q in 0..n {
        while z.get(k + 1).copied().unwrap_or(INF) < q as f32 {
            k += 1;
        }
        let p = v.get(k).copied().unwrap_or(0);
        let delta = q as f32 - p as f32;
        if let Some(slot) = d.get_mut(q) {
            *slot = (delta * delta + f.get(p).copied().unwrap_or(INF)).min(INF);
        }
    }
}

/// Euclidean distance in pixels from every pixel centre to the nearest seed pixel.
fn edt(w: usize, h: usize, seed: impl Fn(usize) -> bool) -> Vec<f32> {
    let mut grid = vec![INF; w.saturating_mul(h)];
    for (i, value) in grid.iter_mut().enumerate() {
        if seed(i) {
            *value = 0.0;
        }
    }
    let n = w.max(h);
    let mut f = vec![0.0f32; n];
    let mut d = vec![0.0f32; n];
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f32; n.saturating_add(1)];
    for x in 0..w {
        for y in 0..h {
            if let Some(slot) = f.get_mut(y) {
                *slot = y
                    .checked_mul(w)
                    .and_then(|row| row.checked_add(x))
                    .and_then(|i| grid.get(i))
                    .copied()
                    .unwrap_or(INF);
            }
        }
        if let (Some(fi), Some(di), Some(vi), Some(zi)) = (
            f.get_mut(..h),
            d.get_mut(..h),
            v.get_mut(..h),
            z.get_mut(..h.saturating_add(1)),
        ) {
            edt_1d(fi, di, vi, zi);
        }
        for y in 0..h {
            if let (Some(value), Some(distance)) = (
                y.checked_mul(w)
                    .and_then(|row| row.checked_add(x))
                    .and_then(|i| grid.get_mut(i)),
                d.get(y),
            ) {
                *value = *distance;
            }
        }
    }
    for y in 0..h {
        if let Some(source) = y
            .checked_mul(w)
            .and_then(|row| grid.get(row..row.checked_add(w)?))
            .and_then(|src| f.get_mut(..w).map(|dst| (src, dst)))
        {
            source.1.copy_from_slice(source.0);
        }
        if let (Some(fi), Some(di), Some(vi), Some(zi)) = (
            f.get_mut(..w),
            d.get_mut(..w),
            v.get_mut(..w),
            z.get_mut(..w.saturating_add(1)),
        ) {
            edt_1d(fi, di, vi, zi);
        }
        for x in 0..w {
            if let (Some(value), Some(distance)) = (
                y.checked_mul(w)
                    .and_then(|row| row.checked_add(x))
                    .and_then(|i| grid.get_mut(i)),
                d.get(x),
            ) {
                *value = distance.sqrt();
            }
        }
    }
    grid
}

/// Approximate signed distance to the shape edge: negative inside, positive outside, refined from
/// fractional coverage at the edge.
pub fn signed_distance(mask: &Mask) -> Vec<f32> {
    let (w, h) = (mask.w, mask.h);
    let outside = edt(w, h, |i| mask.a.get(i).is_some_and(|c| *c >= 0.5));
    let inside = edt(w, h, |i| mask.a.get(i).is_some_and(|c| *c < 0.5));
    (0..w.saturating_mul(h))
        .map(|i| {
            let coverage = mask.a.get(i).copied().unwrap_or(0.0);
            let out = outside.get(i).copied().unwrap_or(0.0);
            let inside_distance = inside.get(i).copied().unwrap_or(0.0);
            if coverage > 0.0 && coverage < 1.0 {
                0.5 - coverage
            } else if coverage >= 0.5 {
                -(inside_distance - 0.5).max(0.0)
            } else {
                (out - 0.5).max(0.0)
            }
        })
        .collect()
}

/// Stroke coverage of `width` pixels around the shape in `mask`.
pub fn stroke(mask: &Mask, width: f32, kind: StrokeKind) -> Mask {
    let distance = signed_distance(mask);
    let a = distance
        .iter()
        .zip(&mask.a)
        .map(|(&d, &coverage)| {
            let value = match kind {
                StrokeKind::Outer => (width + 0.5 - d).clamp(0.0, 1.0).max(coverage),
                StrokeKind::Center => (width / 2.0 + 0.5 - d.abs()).clamp(0.0, 1.0),
                StrokeKind::Inner => (width + 0.5 + d).clamp(0.0, 1.0).min(coverage),
            };
            value.clamp(0.0, 1.0)
        })
        .collect();
    Mask {
        w: mask.w,
        h: mask.h,
        a,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::{Path, fill};

    #[test]
    fn distance_transform_is_euclidean() {
        let d = edt(9, 9, |i| i == 4 * 9 + 4);
        assert_eq!(d.get(4 * 9 + 4).copied(), Some(0.0));
        assert!((d.get(4 * 9 + 7).copied().unwrap_or(0.0) - 3.0).abs() < 1e-5);
        assert!((d.first().copied().unwrap_or(0.0) - (32.0f32).sqrt()).abs() < 1e-4);
        let none = edt(3, 3, |_| false);
        assert!(none.iter().all(|&v| v > 1e5));
    }

    #[test]
    fn strokes_sit_outside_centre_and_inside() {
        let mask = fill(&Path::rect(10.0, 10.0, 30.0, 30.0), 40, 40, 0.0, 0.0);
        let outer = stroke(&mask, 4.0, StrokeKind::Outer);
        assert!(outer.get(7, 20) > 0.99, "outer covers 3px outside");
        assert!(outer.get(4, 20) < 0.01);
        assert!(outer.get(20, 20) > 0.99, "outer includes the fill area");
        let inner = stroke(&mask, 4.0, StrokeKind::Inner);
        assert!(inner.get(12, 20) > 0.99 && inner.get(7, 20) < 0.01 && inner.get(20, 20) < 0.01);
        let centre = stroke(&mask, 4.0, StrokeKind::Center);
        assert!(centre.get(9, 20) > 0.99 && centre.get(11, 20) > 0.99);
        assert!(centre.get(20, 20) < 0.01 && centre.get(5, 20) < 0.01);
    }
}
