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
fn edt(
    w: usize,
    h: usize,
    seed: impl Fn(usize) -> bool,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<f32>, crate::RenderError> {
    check(cancelled)?;
    let mut grid = buffer(w.saturating_mul(h), INF)?;
    for (i, value) in grid.iter_mut().enumerate() {
        if i.is_multiple_of(w) {
            check(cancelled)?;
        }
        if seed(i) {
            *value = 0.0;
        }
    }
    let n = w.max(h);
    let mut f = buffer(n, 0.0f32)?;
    let mut d = buffer(n, 0.0f32)?;
    let mut v = buffer(n, 0usize)?;
    let mut z = buffer(n.saturating_add(1), 0.0f32)?;
    for x in 0..w {
        check(cancelled)?;
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
        check(cancelled)?;
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
    Ok(grid)
}

/// Approximate signed distance to the shape edge: negative inside, positive outside, refined from
/// fractional coverage at the edge.
fn signed_distance(
    mask: &Mask,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<f32>, crate::RenderError> {
    let (w, h) = (mask.w, mask.h);
    let outside = edt(
        w,
        h,
        |i| mask.a.get(i).is_some_and(|c| *c >= 0.5),
        cancelled,
    )?;
    let inside = edt(w, h, |i| mask.a.get(i).is_some_and(|c| *c < 0.5), cancelled)?;
    let mut out = buffer(w.saturating_mul(h), 0.0)?;
    for (i, value) in out.iter_mut().enumerate() {
        if i.is_multiple_of(w) {
            check(cancelled)?;
        }
        let coverage = mask.a.get(i).copied().unwrap_or(0.0);
        let outside_distance = outside.get(i).copied().unwrap_or(0.0);
        let inside_distance = inside.get(i).copied().unwrap_or(0.0);
        *value = if coverage > 0.0 && coverage < 1.0 {
            0.5 - coverage
        } else if coverage >= 0.5 {
            -(inside_distance - 0.5).max(0.0)
        } else {
            (outside_distance - 0.5).max(0.0)
        };
    }
    Ok(out)
}
fn buffer<T: Clone>(size: usize, value: T) -> Result<Vec<T>, crate::RenderError> {
    let mut result = Vec::new();
    result.try_reserve_exact(size)?;
    result.resize(size, value);
    Ok(result)
}
fn check(cancelled: &impl Fn() -> bool) -> Result<(), crate::RenderError> {
    if cancelled() {
        Err(crate::RenderError::Cancelled)
    } else {
        Ok(())
    }
}

/// Signed output-pixel distance from normalized raster coverage: negative inside.
/// Reuses the text outline distance transform. Fractional edge pixels preserve
/// their coverage offset; empty/full rasters saturate beyond the canvas extent.
/// Positive dimensions, <=64 Mi pixels, exact length and finite 0..=1 coverage
/// are required. Cancellation is checked per input row and distance-transform row/column.
/// Callers that enforce a byte budget reserve [`coverage_signed_distance_bytes`]
/// before calling.
pub fn coverage_signed_distance(
    width: u32,
    height: u32,
    coverage: impl ExactSizeIterator<Item = f32>,
    cancelled: impl Fn() -> bool,
) -> Result<Vec<f32>, crate::RenderError> {
    let area = u64::from(width) * u64::from(height);
    if area == 0 || area > crate::raster::MAX_MASK_PIXELS as u64 || coverage.len() as u64 != area {
        return Err(crate::RenderError::InvalidCanvas);
    }
    check(&cancelled)?;
    let mut a = Vec::new();
    a.try_reserve_exact(area as usize)?;
    for (i, c) in coverage.enumerate() {
        if i as u64 >= area {
            return Err(crate::RenderError::InvalidCanvas);
        }
        if i.is_multiple_of(width as usize) {
            check(&cancelled)?;
        }
        if !c.is_finite() || !(0.0..=1.0).contains(&c) {
            return Err(crate::RenderError::InvalidCoverage);
        }
        a.push(c);
    }
    if a.len() as u64 != area {
        return Err(crate::RenderError::InvalidCanvas);
    }
    signed_distance(
        &Mask {
            w: width as usize,
            h: height as usize,
            a,
        },
        &cancelled,
    )
}

/// Peak bytes [`coverage_signed_distance`] holds for a `width` x `height`
/// raster, the returned distances included: the coverage copy, both distance
/// grids and the result (four `f32` per pixel), plus the row scratch of the
/// transform on the longer axis.
pub fn coverage_signed_distance_bytes(width: u32, height: u32) -> u64 {
    const F32: u64 = size_of::<f32>() as u64;
    let area = u64::from(width) * u64::from(height);
    let axis = u64::from(width.max(height));
    // f, d and z are f32 rows (z has one extra slot); v is a usize row.
    let scratch = axis * (3 * F32 + size_of::<usize>() as u64) + F32;
    area * 4 * F32 + scratch
}

/// Stroke coverage of `width` pixels around the shape in `mask`.
pub fn stroke(mask: &Mask, width: f32, kind: StrokeKind) -> Result<Mask, crate::RenderError> {
    let distance = signed_distance(mask, &|| false)?;
    let mut a = buffer(mask.a.len(), 0.0)?;
    for ((output, &d), &coverage) in a.iter_mut().zip(&distance).zip(&mask.a) {
        let value = match kind {
            StrokeKind::Outer => (width + 0.5 - d).clamp(0.0, 1.0).max(coverage),
            StrokeKind::Center => (width / 2.0 + 0.5 - d.abs()).clamp(0.0, 1.0),
            StrokeKind::Inner => (width + 0.5 + d).clamp(0.0, 1.0).min(coverage),
        };
        *output = value.clamp(0.0, 1.0);
    }
    Ok(Mask {
        w: mask.w,
        h: mask.h,
        a,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::{Path, fill};

    #[test]
    fn raster_distance_preserves_edges_and_refuses_invalid_inputs() {
        let mut coverage = vec![0.; 81];
        *coverage.get_mut(40).expect("center") = 1.;
        let d =
            coverage_signed_distance(9, 9, coverage.iter().copied(), || false).expect("distance");
        assert_eq!(d.get(40).copied(), Some(-0.5));
        assert!((d.get(43).copied().unwrap_or(0.) - 2.5).abs() < 1e-5);
        assert!((d.first().copied().unwrap_or(0.) - ((32.0f32).sqrt() - 0.5)).abs() < 1e-4);
        for value in [0., 0.25, 0.5, 0.75, 1.] {
            let distance =
                coverage_signed_distance(1, 1, std::iter::once(value), || false).expect("fraction");
            let d = distance.first().copied().expect("pixel");
            if value > 0. && value < 1. {
                assert_eq!(d, 0.5 - value);
            } else {
                assert!(d.abs() > 1e5);
            }
        }
        for (w, h, pixels) in [
            (0, 1, vec![]),
            (1, 0, vec![]),
            (2, 2, vec![0.; 3]),
            (1, 1, vec![f32::NAN]),
            (1, 1, vec![f32::INFINITY]),
            (1, 1, vec![-0.1]),
            (1, 1, vec![1.1]),
        ] {
            assert!(coverage_signed_distance(w, h, pixels.into_iter(), || false).is_err());
        }
        // Four f32 grids per pixel, three f32 rows and one usize row on the
        // longer axis, and the extra slot of `z`.
        let row = 12 + size_of::<usize>() as u64;
        for (w, h, bytes) in [
            (9, 9, 81 * 16 + 9 * row + 4),
            (192, 48, 192 * 48 * 16 + 192 * row + 4),
            (1, 7, 7 * 16 + 7 * row + 4),
        ] {
            assert_eq!(coverage_signed_distance_bytes(w, h), bytes);
        }
        let calls = std::cell::Cell::new(0);
        assert!(matches!(
            coverage_signed_distance(9, 9, coverage.into_iter(), || {
                calls.set(calls.get() + 1);
                calls.get() >= 20
            }),
            Err(crate::RenderError::Cancelled)
        ));
    }

    #[test]
    fn strokes_sit_outside_centre_and_inside() {
        let mask = fill(&Path::rect(10.0, 10.0, 30.0, 30.0), 40, 40, 0.0, 0.0);
        let outer = stroke(&mask, 4.0, StrokeKind::Outer).expect("outer");
        assert!(outer.get(7, 20) > 0.99, "outer covers 3px outside");
        assert!(outer.get(4, 20) < 0.01);
        assert!(outer.get(20, 20) > 0.99, "outer includes the fill area");
        let inner = stroke(&mask, 4.0, StrokeKind::Inner).expect("inner");
        assert!(inner.get(12, 20) > 0.99 && inner.get(7, 20) < 0.01 && inner.get(20, 20) < 0.01);
        let centre = stroke(&mask, 4.0, StrokeKind::Center).expect("center");
        assert!(centre.get(9, 20) > 0.99 && centre.get(11, 20) > 0.99);
        assert!(centre.get(20, 20) < 0.01 && centre.get(5, 20) < 0.01);
    }
}
