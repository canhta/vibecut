//! Pixel maths shared by the transition kinds: sampling, noise, light, motion blur and blurs.
use super::Tx;
use crate::{Error, ExecutionHooks, Image, MemoryKind, Result, check};

pub(super) const PI: f32 = std::f32::consts::PI;
pub(super) const TAU: f32 = std::f32::consts::TAU;
pub(super) const FRAC_PI_2: f32 = std::f32::consts::FRAC_PI_2;

/// Clamp a computed pixel into the working-image contract.
pub(super) fn working(mut pixel: [f32; 4]) -> [f32; 4] {
    for value in &mut pixel {
        *value = if value.is_finite() {
            value.clamp(0., 1.)
        } else {
            0.
        };
    }
    let alpha = pixel.get(3).copied().unwrap_or(0.);
    for value in pixel.iter_mut().take(3) {
        *value = value.min(alpha);
    }
    pixel
}

#[inline]
pub(super) fn mix(a: [f32; 4], b: [f32; 4], k: f32) -> [f32; 4] {
    std::array::from_fn(|i| {
        let (l, r) = (
            a.get(i).copied().unwrap_or(0.),
            b.get(i).copied().unwrap_or(0.),
        );
        l + (r - l) * k
    })
}

/// Premultiplied `src` over `dst`.
#[inline]
pub(super) fn over(dst: [f32; 4], src: [f32; 4]) -> [f32; 4] {
    let k = 1.0 - src.get(3).copied().unwrap_or(0.);
    std::array::from_fn(|i| {
        src.get(i).copied().unwrap_or(0.) + dst.get(i).copied().unwrap_or(0.) * k
    })
}

#[inline]
pub(super) fn scale4(c: [f32; 4], k: f32) -> [f32; 4] {
    c.map(|v| v * k)
}

/// Add light `rgb × k`, raising alpha towards opaque by the same amount.
#[inline]
pub(super) fn add_light(c: [f32; 4], rgb: [f32; 4], k: f32) -> [f32; 4] {
    std::array::from_fn(|i| {
        let v = c.get(i).copied().unwrap_or(0.);
        if i == 3 {
            (v + k.max(0.)).min(1.)
        } else {
            v + rgb.get(i).copied().unwrap_or(0.) * k
        }
    })
}

#[inline]
pub(super) fn luma(c: [f32; 4]) -> f32 {
    0.2126 * c.first().copied().unwrap_or(0.)
        + 0.7152 * c.get(1).copied().unwrap_or(0.)
        + 0.0722 * c.get(2).copied().unwrap_or(0.)
}

/// Perceptual (about gamma 2.2) luma of a premultiplied linear pixel, 0..1.
#[inline]
pub(super) fn perceptual_luma(c: [f32; 4]) -> f32 {
    luma(c).clamp(0., 1.).powf(1. / 2.2)
}

#[inline]
pub(super) fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    if e1 <= e0 {
        return if x < e0 { 0. } else { 1. };
    }
    let t = ((x - e0) / (e1 - e0)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}

/// 0 at both ends, 1 at the midpoint (smooth).
#[inline]
pub(super) fn bell(p: f32) -> f32 {
    let t = 4. * p * (1. - p);
    t * t * (3. - 2. * t)
}

/// The A to B mix weight for transitions that distort out and then in.
#[inline]
pub(super) fn mid_mix(p: f32) -> f32 {
    smoothstep(0.3, 0.7, p)
}

#[inline]
pub(super) fn ease_out(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(3)
}

#[inline]
pub(super) fn ease_in(t: f32) -> f32 {
    t.clamp(0., 1.).powi(3)
}

/// Integer hash in 0..1.
#[inline]
pub(super) fn hash(x: i32, y: i32, s: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8DA6_B343)
        ^ (y as u32).wrapping_mul(0xD816_3841)
        ^ s.wrapping_mul(0xCB1A_B31F)
        ^ 0x9E37_79B9;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// Smooth value noise in 0..1 with unit cell size.
pub(super) fn vnoise(x: f32, y: f32, s: u32) -> f32 {
    let (xi, yi) = (x.floor(), y.floor());
    let (fx, fy) = (x - xi, y - yi);
    let (ux, uy) = (fx * fx * (3. - 2. * fx), fy * fy * (3. - 2. * fy));
    let (i, j) = (xi as i32, yi as i32);
    let a = hash(i, j, s);
    let b = hash(i + 1, j, s);
    let c = hash(i, j + 1, s);
    let d = hash(i + 1, j + 1, s);
    let top = a + (b - a) * ux;
    let bot = c + (d - c) * ux;
    top + (bot - top) * uy
}

/// Fractal (4-octave) value noise in about 0..1.
pub(super) fn fbm(x: f32, y: f32, s: u32) -> f32 {
    let (mut v, mut amp, mut f, mut norm) = (0., 0.5, 1., 0.);
    for o in 0..4 {
        v += vnoise(x * f, y * f, s.wrapping_add(o * 101)) * amp;
        norm += amp;
        amp *= 0.5;
        f *= 2.03;
    }
    v / norm
}

/// Bilinear sample clamped to the image edges; pixel centres are at +0.5.
pub(super) fn sample_clamped(image: &Image, x: f32, y: f32) -> [f32; 4] {
    let width = image.width;
    let height = image.height;
    let fx = (x - 0.5).clamp(0., width as f32 - 1.);
    let fy = (y - 0.5).clamp(0., height as f32 - 1.);
    let x0 = fx.floor() as usize;
    let y0 = fy.floor() as usize;
    let x1 = (x0 + 1).min(width as usize - 1);
    let y1 = (y0 + 1).min(height as usize - 1);
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let at = |x: usize, y: usize| {
        image
            .pixels
            .get(y * width as usize + x)
            .copied()
            .unwrap_or([0.; 4])
    };
    let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
    let pixel: [f32; 4] = std::array::from_fn(|k| {
        let top = a.get(k).copied().unwrap_or(0.)
            + (b.get(k).copied().unwrap_or(0.) - a.get(k).copied().unwrap_or(0.)) * tx;
        let bottom = c.get(k).copied().unwrap_or(0.)
            + (d.get(k).copied().unwrap_or(0.) - c.get(k).copied().unwrap_or(0.)) * tx;
        top + (bottom - top) * ty
    });
    working(pixel)
}

/// Nearest pixel clamped to the image edges.
pub(super) fn pixel_clamped(image: &Image, x: f32, y: f32) -> [f32; 4] {
    let x = (x as isize).clamp(0, image.width as isize - 1) as usize;
    let y = (y as isize).clamp(0, image.height as isize - 1) as usize;
    image
        .pixels
        .get(y * image.width as usize + x)
        .copied()
        .unwrap_or([0.; 4])
}

/// Average of `n` samples along the segment centred on `(x, y)` with half-extent `(dx, dy)`.
pub(super) fn line_blur(image: &Image, x: f32, y: f32, dx: f32, dy: f32, n: usize) -> [f32; 4] {
    if n <= 1 || (dx.abs() + dy.abs()) < 0.5 {
        return sample_clamped(image, x, y);
    }
    let mut acc = [0.; 4];
    for i in 0..n {
        let t = i as f32 / (n - 1) as f32 * 2. - 1.;
        let s = sample_clamped(image, x + dx * t, y + dy * t);
        for (a, v) in acc.iter_mut().zip(s) {
            *a += v;
        }
    }
    scale4(acc, 1. / n as f32)
}

/// Radial streak blur towards `(cx, cy)`.
pub(super) fn zoom_blur(
    image: &Image,
    x: f32,
    y: f32,
    cx: f32,
    cy: f32,
    k: f32,
    n: usize,
) -> [f32; 4] {
    let (dx, dy) = ((x - cx) * k * 0.5, (y - cy) * k * 0.5);
    line_blur(image, x - dx, y - dy, dx, dy, n)
}

/// Spin blur about `(cx, cy)` over `ang` radians, centred on the pixel.
pub(super) fn spin_blur(
    image: &Image,
    x: f32,
    y: f32,
    cx: f32,
    cy: f32,
    ang: f32,
    n: usize,
) -> [f32; 4] {
    if n <= 1 || ang.abs() < 1e-4 {
        return sample_clamped(image, x, y);
    }
    let (rx, ry) = (x - cx, y - cy);
    let mut acc = [0.; 4];
    for i in 0..n {
        let a = (i as f32 / (n - 1) as f32 - 0.5) * ang;
        let (s, c) = a.sin_cos();
        let p = sample_clamped(image, cx + rx * c - ry * s, cy + rx * s + ry * c);
        for (a, v) in acc.iter_mut().zip(p) {
            *a += v;
        }
    }
    scale4(acc, 1. / n as f32)
}

/// Golden-angle disc sample offsets for a bokeh-like blur of radius `r`; empty for no blur.
pub(super) fn disc_offsets(r: f32, n: usize) -> Vec<(f32, f32)> {
    if r < 0.5 || n <= 1 {
        return Vec::new();
    }
    (0..n)
        .map(|i| {
            let rr = r * ((i as f32 + 0.5) / n as f32).sqrt();
            let a = i as f32 * 2.399_963;
            (rr * a.cos(), rr * a.sin())
        })
        .collect()
}

pub(super) fn disc_blur_with(image: &Image, x: f32, y: f32, offsets: &[(f32, f32)]) -> [f32; 4] {
    if offsets.is_empty() {
        return sample_clamped(image, x, y);
    }
    let mut acc = [0.; 4];
    for &(dx, dy) in offsets {
        let p = sample_clamped(image, x + dx, y + dy);
        for (a, v) in acc.iter_mut().zip(p) {
            *a += v;
        }
    }
    scale4(acc, 1. / offsets.len() as f32)
}

/// Sample count for a blur of extent `len` pixels.
#[inline]
pub(super) fn taps(len: f32) -> usize {
    ((len.abs() * 0.5).ceil() as usize).clamp(1, 40)
}

/// Motion-blur settings from the `motion_blur` parameter: shutter in progress units and samples.
pub(super) fn shutter(t: &Tx, max: f32) -> Result<(f32, usize)> {
    let mb = t.params.fraction("motion_blur")?.clamp(0., 1.);
    Ok(if mb <= 0. {
        (0., 1)
    } else {
        // Keep the middle shutter, but shrink its support to the endpoint.
        // A fixed shutter would retain distorted samples as progress tends to 0/1.
        (
            (max * mb).min(2. * t.progress.min(1. - t.progress)),
            ((mb * 12.).ceil() as usize).clamp(2, 12),
        )
    })
}

/// Average a per-pixel scene over `n` time samples spread over `shutter` around `p`.
pub(super) fn temporal(
    x: f32,
    y: f32,
    p: f32,
    shutter: f32,
    n: usize,
    f: &(impl Fn(f32, f32, f32) -> [f32; 4] + Sync),
) -> [f32; 4] {
    if n <= 1 || shutter <= 1e-5 {
        return f(x, y, p);
    }
    let mut acc = [0.; 4];
    for i in 0..n {
        let q = (p + (i as f32 / (n - 1) as f32 - 0.5) * shutter).clamp(0., 1.);
        let c = f(x, y, q);
        for (a, v) in acc.iter_mut().zip(c) {
            *a += v;
        }
    }
    scale4(acc, 1. / n as f32)
}

/// Unit direction of travel for an angle parameter (0 degrees = up, 90 = right).
pub(super) fn angle_dir(deg: f32) -> (f32, f32) {
    let r = deg.to_radians();
    (r.sin(), -r.cos())
}

/// The centre point in pixels; the stored fraction names the frame origin at top-left.
pub(super) fn centre(t: &Tx) -> Result<(f32, f32)> {
    let [x, y] = t.params.point("center")?;
    Ok((x * t.width as f32, y * t.height as f32))
}

/// A pixel-size parameter in the pixels of these frames.
pub(super) fn px(t: &Tx, id: &str) -> Result<f32> {
    let value = f64::from(t.params.num(id)?.max(0.)) * f64::from(t.scale);
    if !value.is_finite() || value > f64::from(f32::MAX) {
        return Err(Error::Invalid("transition pixel extent"));
    }
    Ok(value as f32)
}

/// An angle parameter in degrees.
pub(super) fn angle_deg(t: &Tx, id: &str) -> Result<f32> {
    Ok(t.params.angle(id)?.to_degrees())
}

/// The reference size for resolution-independent defaults (1 at 1080p height).
pub(super) fn unit(t: &Tx) -> f32 {
    t.height as f32 / 1080.
}

/// A copy of `source` with `other` mixed in at `k`.
pub(super) fn mixed(
    source: &Image,
    other: &Image,
    k: f32,
    hooks: &dyn ExecutionHooks,
) -> Result<Image> {
    if (source.width, source.height) != (other.width, other.height) {
        return Err(Error::Invalid("transition frame size"));
    }
    let mut out = Image::new(source.width, source.height, [0.; 4], hooks)?;
    let width = source.width as usize;
    for (n, ((output, a), b)) in out
        .pixels
        .chunks_mut(width)
        .zip(source.pixels.chunks(width))
        .zip(other.pixels.chunks(width))
        .enumerate()
    {
        if n % 8 == 0 {
            check(hooks)?;
        }
        for ((px, a), b) in output.iter_mut().zip(a).zip(b) {
            *px = working(mix(*a, *b, k));
        }
    }
    Ok(out)
}

/// A Gaussian-blurred copy (box-pass approximation, sigma in pixels).
pub(super) fn blurred(image: &Image, sigma: f32, hooks: &dyn ExecutionHooks) -> Result<Image> {
    check(hooks)?;
    if !sigma.is_finite() || sigma < 0. {
        return Err(Error::Invalid("transition blur radius"));
    }
    if sigma <= 0.3 {
        return image.copy(hooks);
    }
    let radii_x = boxes_for_gauss(sigma.min(image.width.max(8) as f32 * 2.));
    let radii_y = boxes_for_gauss(sigma.min(image.height.max(8) as f32 * 2.));
    let mut source = image.copy(hooks)?;
    let mut destination = Image::new(image.width, image.height, [0.; 4], hooks)?;
    for radius in &radii_x {
        blur_rows(&source, &mut destination, *radius, hooks)?;
        std::mem::swap(&mut source, &mut destination);
    }
    for radius in &radii_y {
        blur_columns(&source, &mut destination, *radius, hooks)?;
        std::mem::swap(&mut source, &mut destination);
    }
    Ok(source)
}

/// Box radii for an n-pass box blur approximating a Gaussian of sigma.
fn boxes_for_gauss(sigma: f32) -> [usize; 3] {
    // Axis caps are derived from Image's area bound. f64 avoids integer-square
    // overflow even at the largest admitted axis, without a host-specific cap.
    let sigma = f64::from(sigma);
    let wideal = (4. * sigma * sigma + 1.).sqrt();
    let mut wl = wideal.floor() as usize;
    if wl.is_multiple_of(2) {
        wl -= 1;
    }
    let wu = wl + 2;
    let lower = wl as f64;
    let mideal = (12. * sigma * sigma - 3. * lower * lower - 12. * lower - 9.) / (-4. * lower - 4.);
    let m = mideal.round().clamp(0., 3.) as usize;
    std::array::from_fn(|i| ((if i < m { wl } else { wu }) - 1) / 2)
}

/// Running box filter along rows with repeat-edge borders.
fn blur_rows(
    source: &Image,
    destination: &mut Image,
    radius: usize,
    hooks: &dyn ExecutionHooks,
) -> Result<()> {
    let width = source.width as usize;
    check(hooks)?;
    if radius == 0 || width == 0 {
        destination.pixels.copy_from_slice(&source.pixels);
        return Ok(());
    }
    let inverse = 1. / (2 * radius + 1) as f32;
    let clamp = |i: isize| i.clamp(0, width as isize - 1) as usize;
    for (y, (output, input)) in destination
        .pixels
        .chunks_mut(width)
        .zip(source.pixels.chunks(width))
        .enumerate()
    {
        if y % 8 == 0 {
            check(hooks)?;
        }
        let mut acc = [0.; 4];
        for i in -(radius as isize)..=(radius as isize) {
            if i.rem_euclid(256) == 0 {
                check(hooks)?;
            }
            let sample = input.get(clamp(i)).copied().unwrap_or([0.; 4]);
            for (a, v) in acc.iter_mut().zip(sample) {
                *a += v;
            }
        }
        for (x, out) in output.iter_mut().enumerate() {
            if x.is_multiple_of(4096) {
                check(hooks)?;
            }
            *out = working(scale4(acc, inverse));
            let leaving = input
                .get(clamp(x as isize - radius as isize))
                .copied()
                .unwrap_or([0.; 4]);
            let entering = input
                .get(clamp(x as isize + radius as isize + 1))
                .copied()
                .unwrap_or([0.; 4]);
            for (a, (l, e)) in acc.iter_mut().zip(leaving.into_iter().zip(entering)) {
                *a = *a - l + e;
            }
        }
    }
    Ok(())
}

/// Running box filter down columns with repeat-edge borders.
fn blur_columns(
    source: &Image,
    destination: &mut Image,
    radius: usize,
    hooks: &dyn ExecutionHooks,
) -> Result<()> {
    let width = source.width as usize;
    let height = source.height as usize;
    check(hooks)?;
    if radius == 0 || height == 0 {
        destination.pixels.copy_from_slice(&source.pixels);
        return Ok(());
    }
    let bytes = (width as u64)
        .checked_mul(16)
        .ok_or(Error::Invalid("transition scratch size"))?;
    let _lease = hooks.reserve(MemoryKind::Compositor, bytes)?;
    let mut acc = Vec::new();
    acc.try_reserve_exact(width)?;
    while acc.len() < width {
        check(hooks)?;
        acc.resize((acc.len() + 4096).min(width), [0.; 4]);
    }
    let clamp = |i: isize| i.clamp(0, height as isize - 1) as usize;
    for i in -(radius as isize)..=(radius as isize) {
        if i.rem_euclid(8) == 0 {
            check(hooks)?;
        }
        let y = clamp(i);
        let row = source
            .pixels
            .get(y * width..y * width + width)
            .ok_or(Error::Invalid("transition blur row"))?;
        for (x, (a, v)) in acc.iter_mut().zip(row).enumerate() {
            if x.is_multiple_of(4096) {
                check(hooks)?;
            }
            for (a, v) in a.iter_mut().zip(*v) {
                *a += v;
            }
        }
    }
    let inverse = 1. / (2 * radius + 1) as f32;
    for y in 0..height {
        if y % 8 == 0 {
            check(hooks)?;
        }
        let leaving = source
            .pixels
            .get(clamp(y as isize - radius as isize) * width..)
            .ok_or(Error::Invalid("transition blur row"))?;
        let entering = source
            .pixels
            .get(clamp(y as isize + radius as isize + 1) * width..)
            .ok_or(Error::Invalid("transition blur row"))?;
        let output = destination
            .pixels
            .get_mut(y * width..y * width + width)
            .ok_or(Error::Invalid("transition blur row"))?;
        for (x, ((out, a), (l, e))) in output
            .iter_mut()
            .zip(acc.iter_mut())
            .zip(leaving.iter().zip(entering))
            .enumerate()
        {
            if x.is_multiple_of(4096) {
                check(hooks)?;
            }
            *out = working(scale4(*a, inverse));
            for (v, (l, e)) in a.iter_mut().zip(l.iter().zip(e)) {
                *v = *v - l + e;
            }
        }
    }
    Ok(())
}
