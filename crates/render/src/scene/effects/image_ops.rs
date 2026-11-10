//! Image helpers shared by whole-image effect ops: edge-aware bilinear sampling,
//! a gaussian blur at a given sigma and the downsample/upsample pair.
use super::{CpuEffects, EffectCompositor, EffectInputs, sigma_passes};
use crate::{ExecutionHooks, Image, Result};

/// How a sample outside the image is taken.
#[allow(dead_code, reason = "Mirror and Wrap are used by the 264 and 265 ops")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Edge {
    /// Repeat the border pixel.
    Clamp,
    /// Repeat the picture reflected at its border.
    Mirror,
    /// Repeat the picture on the opposite side.
    Wrap,
    /// Outside the picture is fully transparent.
    Transparent,
}

fn axis(i: i64, n: i64, edge: Edge) -> i64 {
    if n <= 1 && edge != Edge::Transparent {
        return 0;
    }
    match edge {
        Edge::Transparent => i,
        Edge::Clamp => i.clamp(0, n - 1),
        Edge::Wrap => i.rem_euclid(n),
        Edge::Mirror => {
            let period = 2 * (n - 1);
            let m = i.rem_euclid(period);
            if m >= n { period - m } else { m }
        }
    }
}

/// Bilinear sample in texel-index coordinates: integer `(x, y)` is that exact texel.
/// Geometry in image-edge coordinates (pixel centres `x+0.5`, `y+0.5`) subtracts
/// 0.5 exactly once before this lookup, including reflection and upsampling.
pub(super) fn sample(image: &Image, x: f32, y: f32, edge: Edge) -> [f32; 4] {
    if !x.is_finite() || !y.is_finite() {
        return [0.0; 4];
    }
    let (w, h) = (i64::from(image.width), i64::from(image.height));
    let (floor_x, floor_y) = (x.floor(), y.floor());
    let (tx, ty) = (x - floor_x, y - floor_y);
    let (ix, iy) = (floor_x as i64, floor_y as i64);
    let at = |dx: i64, dy: i64| {
        let Some((x, y)) = ix.checked_add(dx).zip(iy.checked_add(dy)) else {
            return [0.0; 4];
        };
        let (sx, sy) = (axis(x, w, edge), axis(y, h, edge));
        if sx < 0 || sy < 0 || sx >= w || sy >= h {
            return [0.0; 4];
        }
        image
            .pixels
            .get((sy * w + sx) as usize)
            .copied()
            .unwrap_or([0.0; 4])
    };
    let corners = [at(0, 0), at(1, 0), at(0, 1), at(1, 1)];
    std::array::from_fn(|k| {
        let channel = |i: usize| {
            corners
                .get(i)
                .and_then(|c| c.get(k))
                .copied()
                .unwrap_or(0.0)
        };
        let [a, b, c, d] = [channel(0), channel(1), channel(2), channel(3)];
        let top = a + (b - a) * tx;
        let bottom = c + (d - c) * tx;
        (top + (bottom - top) * ty).clamp(0.0, 1.0)
    })
}

/// Gaussian blur of `image` at `sigma` output pixels; sigma <= 0.01 is a copy.
#[allow(dead_code, reason = "first used by the 264 and 265 ops")]
pub(super) fn blur(image: &Image, sigma: f32, hooks: &dyn ExecutionHooks) -> Result<Image> {
    let passes = sigma_passes(sigma, image.width, image.height)?;
    if passes.is_empty() {
        return image.copy(hooks);
    }
    CpuEffects.apply(
        image,
        &passes,
        EffectInputs {
            input: image,
            coverage: None,
            below: image,
        },
        hooks,
    )
}

/// Box half-resolution downsample: 2x2 average, odd edges use the pixels present.
#[allow(dead_code, reason = "first used by the 264 and 265 ops")]
pub(super) fn downsample(image: &Image, hooks: &dyn ExecutionHooks) -> Result<Image> {
    image.half(hooks)
}

/// Bilinear upsample to `width` x `height` with edge clamping.
#[allow(dead_code, reason = "first used by the 264 and 265 ops")]
pub(super) fn upsample(
    image: &Image,
    width: u32,
    height: u32,
    hooks: &dyn ExecutionHooks,
) -> Result<Image> {
    if image.width == width && image.height == height {
        return image.copy(hooks);
    }
    let stride = width as usize;
    let (scale_x, scale_y) = (
        image.width as f32 / width as f32,
        image.height as f32 / height as f32,
    );
    let mut out = Image::new(width, height, [0.0; 4], hooks)?;
    crate::image::row_chunks(&mut out.pixels, stride, hooks, |start, rows| {
        for (row, pixels) in rows.chunks_mut(stride).enumerate() {
            crate::check(hooks)?;
            let y = (start / stride + row) as f32 + 0.5;
            for (x, pixel) in pixels.iter_mut().enumerate() {
                let x = x as f32 + 0.5;
                *pixel = sample(image, x * scale_x - 0.5, y * scale_y - 0.5, Edge::Clamp);
            }
        }
        Ok(())
    })?;
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    struct Hooks;
    struct Lease;
    impl crate::Reservation for Lease {}
    impl ExecutionHooks for Hooks {
        fn cancelled(&self) -> bool {
            false
        }
        fn reserve(&self, _: crate::MemoryKind, _: u64) -> Result<Box<dyn crate::Reservation>> {
            Ok(Box::new(Lease))
        }
    }
    #[test]
    fn hostile_sample_coordinates_stay_finite_without_index_overflow() {
        let image = Image::new(1, 1, [0.5, 0.0, 0.0, 1.0], &Hooks).unwrap();
        for edge in [Edge::Clamp, Edge::Mirror, Edge::Wrap, Edge::Transparent] {
            for coordinate in [f32::MAX, -f32::MAX] {
                let pixel = sample(&image, coordinate, coordinate, edge);
                assert!(
                    pixel
                        .iter()
                        .all(|v| v.is_finite() && *v >= 0.0 && *v <= pixel[3])
                );
            }
            for coordinate in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                assert_eq!(sample(&image, coordinate, 0.0, edge), [0.0; 4]);
                assert_eq!(sample(&image, 0.0, coordinate, edge), [0.0; 4]);
            }
        }
    }
    #[test]
    fn singleton_and_strip_edges_keep_exact_and_fractional_semantics() {
        use Edge::*;
        for (w, h) in [(1, 3), (3, 1), (1, 1)] {
            let pixels = if w * h == 1 {
                vec![[0.5, 0.0, 0.0, 1.0]]
            } else {
                [0.25, 0.5, 1.0].map(|r| [r, 0.0, 0.0, 1.0]).to_vec()
            };
            let image = Image::from_working_pixels(w, h, pixels.into_iter(), &Hooks).unwrap();
            for edge in [Clamp, Mirror, Wrap, Transparent] {
                // Fixed strip-centre value .5, move along its singleton axis.
                for (coordinate, transparent_weight) in
                    [(-1.0, 0.0), (-0.5, 0.5), (0.0, 1.0), (0.5, 0.5), (1.0, 0.0)]
                {
                    let (x, y) = if w == 1 {
                        (coordinate, if h == 1 { 0.0 } else { 1.0 })
                    } else {
                        (1.0, coordinate)
                    };
                    let weight = if edge == Transparent {
                        transparent_weight
                    } else {
                        1.0
                    };
                    assert_eq!(
                        sample(&image, x, y, edge),
                        [0.5 * weight, 0.0, 0.0, weight],
                        "{w}x{h} {edge:?} {x},{y}"
                    );
                }
            }
            if w * h == 1 {
                assert_eq!(
                    sample(&image, -0.5, -0.5, Transparent),
                    [0.125, 0.0, 0.0, 0.25]
                );
                continue;
            }
            // Explicit oracle for the nonuniform three-texel strip, not a second sampler.
            for (edge, red, alpha) in [
                (
                    Clamp,
                    [0.25, 0.25, 0.25, 0.375, 0.5, 0.75, 1.0, 1.0, 1.0, 1.0],
                    [1.0; 10],
                ),
                (
                    Mirror,
                    [0.5, 0.375, 0.25, 0.375, 0.5, 0.75, 1.0, 0.75, 0.5, 0.375],
                    [1.0; 10],
                ),
                (
                    Wrap,
                    [1.0, 0.625, 0.25, 0.375, 0.5, 0.75, 1.0, 0.625, 0.25, 0.375],
                    [1.0; 10],
                ),
                (
                    Transparent,
                    [0.0, 0.125, 0.25, 0.375, 0.5, 0.75, 1.0, 0.5, 0.0, 0.0],
                    [0.0, 0.5, 1.0, 1.0, 1.0, 1.0, 1.0, 0.5, 0.0, 0.0],
                ),
            ] {
                for ((at, r), a) in [-1.0, -0.5, 0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5]
                    .into_iter()
                    .zip(red)
                    .zip(alpha)
                {
                    let (x, y) = if w == 1 { (0.0, at) } else { (at, 0.0) };
                    assert_eq!(
                        sample(&image, x, y, edge),
                        [r, 0.0, 0.0, a],
                        "{w}x{h} {edge:?} {at}"
                    );
                }
            }
        }
    }
}
