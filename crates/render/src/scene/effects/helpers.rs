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
    if n <= 1 {
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

/// Bilinear sample at a pixel-edge coordinate `(x, y)` with `edge` handling.
pub(super) fn sample(image: &Image, x: f32, y: f32, edge: Edge) -> [f32; 4] {
    let (w, h) = (i64::from(image.width), i64::from(image.height));
    let (floor_x, floor_y) = (x.floor(), y.floor());
    let (tx, ty) = (x - floor_x, y - floor_y);
    let (ix, iy) = (floor_x as i64, floor_y as i64);
    let at = |dx: i64, dy: i64| {
        let (sx, sy) = (axis(ix + dx, w, edge), axis(iy + dy, h, edge));
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
        let channel = |i: usize| corners.get(i).and_then(|c| c.get(k)).copied().unwrap_or(0.0);
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
                *pixel = sample(
                    image,
                    x * scale_x - 0.5,
                    y * scale_y - 0.5,
                    Edge::Clamp,
                );
            }
        }
        Ok(())
    })?;
    Ok(out)
}
