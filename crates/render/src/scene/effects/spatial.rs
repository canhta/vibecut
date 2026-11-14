//! CPU references of the four remaining E1 spatial entries: Detail Boost, Lens
//! Blur, Mirror and Wrap Shift.
//!
//! Detail Boost reuses the shared gaussian and `detail_boost` passes. Lens Blur,
//! Mirror and Wrap Shift are one `Sample` pass each; none has a shader entry, so
//! the run splitter keeps them on the CPU.
use super::{
    EffectFrame, EffectGeometry, EffectPass, Error, ExecutionHooks, Image, Result, SampleInputs,
    Uniform, clamp_pixel, encoded_color, image_ops, luma, sigma_passes, straight, uniform,
};
use std::f32::consts::{PI, TAU};

/// Largest Detail Boost gaussian sigma in output pixels. Radii whose clip-pixel
/// extent exceeds it saturate here, which keeps the pass list within the
/// reserved kernel slots (64 iterations of two passes).
const DETAIL_SIGMA_MAX: f32 = 64.0;
/// Detail Boost never goes below the gaussian the kernel builder resolves.
const DETAIL_SIGMA_MIN: f32 = 0.6;
/// Iris polygon sides per `shape` choice index; zero is the circle.
const LENS_SIDES: [u32; 6] = [0, 3, 4, 5, 6, 8];
/// Gather rings of the Lens Blur disc, at most.
const LENS_RINGS: u32 = 12;
/// Gather radius in pixels of the working level: a disc wider than this is
/// gathered on a copy halved (2x2 box) until it fits, then upsampled, so the
/// tap count per output pixel stays bounded (at most 1 + 8 * 6 = 49 taps here).
const LENS_DIRECT_REACH: f32 = 3.0;
/// Largest Lens Blur gather radius in output pixels; farther taps would all
/// clamp to the picture edge anyway.
const LENS_RADIUS_MAX: f32 = 1e6;

fn working_size(frame: &EffectFrame, geometry: &EffectGeometry) -> Result<[f32; 2]> {
    if geometry.working_size() != [frame.width, frame.height] {
        return Err(Error::Invalid("effect spatial geometry"));
    }
    Ok([frame.width as f32, frame.height as f32])
}

/// Detail Boost is an unsharp mask of an authored radius: a gaussian of
/// `radius` clip pixels on the smaller axis gain (between .6 and 64 output
/// pixels), subtracted from the immutable input and added back by `amount`
/// percent for differences at or above `threshold` (0..=255 codes) times
/// coverage. Amount 0 emits no pass.
pub(super) fn detail_boost_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    working_size(frame, geometry)?;
    let amount = frame.scalar("amount")?;
    if amount == 0.0 {
        return Ok(Vec::new());
    }
    let [gx, gy] = geometry.clip_pixel_scale();
    let sigma = (frame.scalar("radius")? * gx.min(gy)) as f32;
    if !sigma.is_finite() {
        return Err(Error::Invalid("effect spatial extent"));
    }
    let sigma = sigma.clamp(DETAIL_SIGMA_MIN, DETAIL_SIGMA_MAX);
    let mut passes = sigma_passes(sigma, frame.width, frame.height)?;
    passes.push(EffectPass::Sample {
        entry_point: "detail_boost",
        uniforms: vec![
            Uniform::Float((amount / 100.0) as f32),
            Uniform::Float((frame.scalar("threshold")? / 255.0) as f32),
        ],
    });
    Ok(passes)
}

/// Lens Blur gathers the current image over an iris-shaped disc on up to twelve
/// rings. Its radius is `amount` clip pixels per axis, so unequal gains make an
/// ellipse. Radii under half an output pixel emit no pass. A disc wider than
/// three pixels is gathered on a copy halved until it fits (the source's
/// halve-and-upsample levels), see [`reduced_sample`]; within three pixels the
/// highlight boost is applied to each tap, wider it is applied to each texel
/// before halving.
pub(super) fn lens_blur_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    working_size(frame, geometry)?;
    let [gx, gy] = geometry.clip_pixel_scale();
    let amount = frame.scalar("amount")?;
    let (rx, ry) = ((amount * gx) as f32, (amount * gy) as f32);
    if rx.is_nan() || ry.is_nan() {
        return Err(Error::Invalid("effect spatial extent"));
    }
    let (rx, ry) = (rx.min(LENS_RADIUS_MAX), ry.min(LENS_RADIUS_MAX));
    let reach = rx.max(ry);
    if reach < 0.5 {
        return Ok(Vec::new());
    }
    let sides = LENS_SIDES
        .get(frame.choice("shape")? as usize)
        .copied()
        .ok_or(Error::Invalid("effect parameter"))?;
    let levels = lens_levels(rx, ry, frame.width, frame.height);
    let level_reach = reach / 2f32.powi(levels as i32);
    let rings = (level_reach.ceil() as u32).clamp(1, LENS_RINGS);
    Ok(vec![EffectPass::Sample {
        entry_point: "lens_blur",
        uniforms: [
            sides as f32,
            frame.scalar("rotation")?.to_radians() as f32,
            rx,
            ry,
            rings as f32,
            (frame.scalar("highlight")? / 100.0 * 6.0) as f32,
            (frame.scalar("threshold")? / 100.0) as f32,
        ]
        .map(Uniform::Float)
        .to_vec(),
    }])
}

/// Halvings of a `width` x `height` picture before the disc of radii `rx`, `ry`
/// fits [`LENS_DIRECT_REACH`] (or the picture is one pixel). Zero gathers at
/// full resolution.
fn lens_levels(rx: f32, ry: f32, width: u32, height: u32) -> u32 {
    let (mut reach, mut extent, mut levels) = (rx.max(ry), width.max(height), 0);
    while reach > LENS_DIRECT_REACH && extent > 1 {
        reach /= 2.0;
        extent = extent.div_ceil(2);
        levels += 1;
    }
    levels
}

pub(super) fn valid_lens_blur(values: &[f32], _: u32, _: u32) -> bool {
    let at = |i| uniform(values, i);
    LENS_SIDES.iter().any(|sides| *sides as f32 == at(0))
        && (0.0..=LENS_RADIUS_MAX).contains(&at(2))
        && (0.0..=LENS_RADIUS_MAX).contains(&at(3))
        && at(4).fract() == 0.0
        && (1.0..=LENS_RINGS as f32).contains(&at(4))
        && (0.0..=6.0).contains(&at(5))
        && (0.0..=1.0).contains(&at(6))
}

/// Circumradius-1 regular polygon radius in direction `theta`; zero sides is a circle.
fn iris_radius(sides: u32, theta: f32, rotation: f32) -> f32 {
    if sides == 0 {
        return 1.0;
    }
    let segment = TAU / sides as f32;
    let a = (theta - rotation).rem_euclid(segment) - segment / 2.0;
    (PI / sides as f32).cos() / a.cos()
}

/// One bilinear tap with the highlight boost: bright straight colours scale by
/// `1 + boost * smoothstep(threshold, 1, encoded luma)`, so bright points keep
/// their energy through the average. A threshold of 1 boosts only pure white.
fn lens_tap(inputs: &SampleInputs<'_>, x: f32, y: f32, boost: f32, threshold: f32) -> [f32; 4] {
    boost_pixel(
        inputs.sample_current(x, y, image_ops::Edge::Clamp),
        boost,
        threshold,
    )
}

/// The highlight boost of one premultiplied pixel (see [`lens_tap`]).
fn boost_pixel(pixel: [f32; 4], boost: f32, threshold: f32) -> [f32; 4] {
    if boost <= 0.0 || pixel[3] <= 1e-6 {
        return pixel;
    }
    let (colour, _) = straight(pixel);
    let l = luma(encoded_color(colour));
    let t = if threshold >= 1.0 {
        if l >= 1.0 { 1.0 } else { 0.0 }
    } else {
        ((l - threshold) / (1.0 - threshold)).clamp(0.0, 1.0)
    };
    let k = 1.0 + boost * t * t * (3.0 - 2.0 * t);
    [pixel[0] * k, pixel[1] * k, pixel[2] * k, pixel[3]]
}

/// Taps on gather ring `ring` (1-based).
fn ring_taps(ring: u32) -> u32 {
    (ring * 8).max(6)
}

/// Offset of tap `i` of ring `ring` out of `rings`, in units of the radii.
fn ring_offset(sides: u32, rotation: f32, rings: u32, ring: u32, i: u32) -> (f32, f32) {
    let radius = ring as f32 / rings as f32;
    let theta = TAU * (i as f32 + 0.5 * (ring % 2) as f32) / ring_taps(ring) as f32;
    let reach = radius * iris_radius(sides, theta, rotation);
    (theta.cos() * reach, theta.sin() * reach)
}

/// Gather at the picture's own resolution, per output pixel; the reference for
/// discs within [`LENS_DIRECT_REACH`]. Wider discs run through
/// [`reduced_sample`] and never reach this op.
pub(super) fn lens_blur(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let sides = uniform(values, 0) as u32;
    let [rotation, rx, ry, rings, boost, threshold] =
        [1, 2, 3, 4, 5, 6].map(|i| uniform(values, i));
    let rings = rings as u32;
    let (cx, cy) = (x as f32, y as f32);
    let mut sum = lens_tap(inputs, cx, cy, boost, threshold);
    let mut count = 1.0f32;
    for ring in 1..=rings {
        for i in 0..ring_taps(ring) {
            let (dx, dy) = ring_offset(sides, rotation, rings, ring, i);
            let tap = lens_tap(inputs, cx + dx * rx, cy + dy * ry, boost, threshold);
            for (total, value) in sum.iter_mut().zip(tap) {
                *total += value;
            }
            count += 1.0;
        }
    }
    sum.map(|v| v / count)
}

/// Bilinear sample of a reduced plane in texel-index coordinates with clamped
/// edges. Planes hold boosted energy, so the result is not clamped.
fn plane_sample(plane: &Image, x: f32, y: f32) -> [f32; 4] {
    if !x.is_finite() || !y.is_finite() {
        return [0.0; 4];
    }
    let (w, h) = (i64::from(plane.width), i64::from(plane.height));
    let (floor_x, floor_y) = (x.floor(), y.floor());
    let (tx, ty) = (x - floor_x, y - floor_y);
    let (ix, iy) = (floor_x as i64, floor_y as i64);
    let at = |dx: i64, dy: i64| {
        let sx = ix.saturating_add(dx).clamp(0, w - 1);
        let sy = iy.saturating_add(dy).clamp(0, h - 1);
        plane
            .pixels
            .get((sy * w + sx) as usize)
            .copied()
            .unwrap_or([0.0; 4])
    };
    let [a, b, c, d] = [at(0, 0), at(1, 0), at(0, 1), at(1, 1)];
    std::array::from_fn(|k| {
        let channel = |p: [f32; 4]| p.get(k).copied().unwrap_or(0.0);
        let (a, b, c, d) = (channel(a), channel(b), channel(c), channel(d));
        let top = a + (b - a) * tx;
        let bottom = c + (d - c) * tx;
        top + (bottom - top) * ty
    })
}

/// The whole-picture form of Lens Blur for a disc wider than
/// [`LENS_DIRECT_REACH`] (`None` for any other pass or a disc that fits): the
/// highlight boost is applied to each texel, the boosted copy is halved by 2x2
/// box averages ([`lens_levels`] times), the unboosted iris gather runs on that
/// small plane with radii and ring count at its scale, and the result is
/// upsampled bilinearly (edge clamp) to the picture. Per output pixel this is
/// at most 49 gather taps on a plane of a 4^levels-th of the pixels.
pub(super) fn reduced_sample(
    entry_point: &str,
    current: &Image,
    values: &[f32],
    hooks: &dyn ExecutionHooks,
) -> Result<Option<Image>> {
    if entry_point != "lens_blur" {
        return Ok(None);
    }
    let sides = uniform(values, 0) as u32;
    let [rotation, rx, ry, rings, boost, threshold] =
        [1, 2, 3, 4, 5, 6].map(|i| uniform(values, i));
    let levels = lens_levels(rx, ry, current.width, current.height);
    if levels == 0 {
        return Ok(None);
    }
    let scale = 2f32.powi(levels as i32);
    let width = current.width as usize;
    let mut low = if boost > 0.0 {
        let mut boosted = Image::new(current.width, current.height, [0.0; 4], hooks)?;
        crate::image::row_chunks(&mut boosted.pixels, width, hooks, |start, rows| {
            crate::check(hooks)?;
            let source = current
                .pixels
                .get(start..start + rows.len())
                .ok_or(Error::Invalid("effect pixel"))?;
            for (out, pixel) in rows.iter_mut().zip(source) {
                *out = boost_pixel(*pixel, boost, threshold);
            }
            Ok(())
        })?;
        boosted.half(hooks)?
    } else {
        current.half(hooks)?
    };
    for _ in 1..levels {
        low = low.half(hooks)?;
    }
    let rings = rings as u32;
    let offsets: Vec<(f32, f32)> = (1..=rings)
        .flat_map(|ring| (0..ring_taps(ring)).map(move |i| (ring, i)))
        .map(|(ring, i)| ring_offset(sides, rotation, rings, ring, i))
        .collect();
    let count = (offsets.len() + 1) as f32;
    let (lx, ly) = (rx / scale, ry / scale);
    let low_width = low.width as usize;
    let mut gathered = Image::new(low.width, low.height, [0.0; 4], hooks)?;
    crate::image::row_chunks(&mut gathered.pixels, low_width, hooks, |start, rows| {
        for (row, pixels) in rows.chunks_mut(low_width).enumerate() {
            crate::check(hooks)?;
            let y = (start / low_width + row) as f32;
            for (x, out) in pixels.iter_mut().enumerate() {
                let x = x as f32;
                let mut sum = plane_sample(&low, x, y);
                for (dx, dy) in &offsets {
                    let tap = plane_sample(&low, x + dx * lx, y + dy * ly);
                    for (total, value) in sum.iter_mut().zip(tap) {
                        *total += value;
                    }
                }
                *out = sum.map(|v| v / count);
            }
        }
        Ok(())
    })?;
    let mut out = Image::new(current.width, current.height, [0.0; 4], hooks)?;
    let (sx, sy) = (
        gathered.width as f32 / current.width as f32,
        gathered.height as f32 / current.height as f32,
    );
    crate::image::row_chunks(&mut out.pixels, width, hooks, |start, rows| {
        for (row, pixels) in rows.chunks_mut(width).enumerate() {
            crate::check(hooks)?;
            let y = ((start / width + row) as f32 + 0.5) * sy - 0.5;
            for (x, pixel) in pixels.iter_mut().enumerate() {
                let x = (x as f32 + 0.5) * sx - 0.5;
                *pixel = clamp_pixel(plane_sample(&gathered, x, y));
            }
        }
        Ok(())
    })?;
    Ok(Some(out))
}

/// Mirror reflects the half of the picture on the positive side of a line
/// through the centre onto the other half, through the shared `mirror` sample
/// (instance input, transparent beyond the frame). The line's normal is
/// `(cos angle, sin angle)`; the centre is a percent of each axis.
pub(super) fn mirror_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    let [w, h] = working_size(frame, geometry)?;
    let angle = frame.scalar("angle")?.to_radians();
    let (sin, cos) = angle.sin_cos();
    let cx = (frame.scalar("center_x")? / 100.0) as f32 * w;
    let cy = (frame.scalar("center_y")? / 100.0) as f32 * h;
    Ok(vec![EffectPass::Sample {
        entry_point: "mirror",
        uniforms: [cx, cy, cos as f32, sin as f32]
            .map(Uniform::Float)
            .to_vec(),
    }])
}

/// Wrap Shift moves the picture by a percent of each axis and wraps what leaves
/// one edge back in at the opposite edge, then blends the original back by
/// `blend` percent. A shift that is a whole frame on both axes, or blend 100,
/// emits no pass. Fractional shifts interpolate across the seam.
pub(super) fn wrap_shift_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    let [w, h] = working_size(frame, geometry)?;
    let shift = |id: &str, extent: f32| -> Result<f32> {
        let fraction = frame.scalar(id)? / 100.0;
        Ok((fraction * f64::from(extent)).rem_euclid(f64::from(extent)) as f32)
    };
    let (dx, dy) = (shift("shift_x", w)?, shift("shift_y", h)?);
    let blend = (frame.scalar("blend")? / 100.0) as f32;
    if blend >= 1.0 || (dx == 0.0 && dy == 0.0) {
        return Ok(Vec::new());
    }
    Ok(vec![EffectPass::Sample {
        entry_point: "wrap_shift",
        uniforms: [dx, dy, blend].map(Uniform::Float).to_vec(),
    }])
}

pub(super) fn valid_wrap_shift(values: &[f32], width: u32, height: u32) -> bool {
    (0.0..=width as f32).contains(&uniform(values, 0))
        && (0.0..=height as f32).contains(&uniform(values, 1))
        && (0.0..=1.0).contains(&uniform(values, 2))
}

pub(super) fn wrap_shift(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let [dx, dy, blend] = [0, 1, 2].map(|i| uniform(values, i));
    let shifted = inputs.sample_current(x as f32 - dx, y as f32 - dy, image_ops::Edge::Wrap);
    let original = inputs.texel_current(x, y);
    std::array::from_fn(|k| {
        let (s, o) = (
            shifted.get(k).copied().unwrap_or(0.0),
            original.get(k).copied().unwrap_or(0.0),
        );
        s + (o - s) * blend
    })
}
