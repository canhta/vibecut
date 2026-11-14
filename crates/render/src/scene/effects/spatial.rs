//! CPU references of the four remaining E1 spatial entries: Detail Boost, Lens
//! Blur, Mirror and Wrap Shift.
//!
//! Detail Boost reuses the shared gaussian and `detail_boost` passes. Lens Blur,
//! Mirror and Wrap Shift are one `Sample` pass each (Lens Blur behind an optional
//! gaussian prefilter); none has a shader entry, so the run splitter keeps them
//! on the CPU.
use super::{
    EffectFrame, EffectGeometry, EffectPass, Error, Result, SampleInputs, Uniform, encoded_color,
    image_ops, luma, sigma_passes, straight, uniform,
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
/// Gather radius in output pixels above which the twelve rings leave gaps and
/// a gaussian prefilter of a twenty-fourth of the radius closes them.
const LENS_PREFILTER_FROM: f32 = 12.0;
/// Largest Lens Blur gather radius in output pixels; farther taps would all
/// clamp to the picture edge anyway.
const LENS_RADIUS_MAX: f32 = 1e6;
/// Largest Lens Blur prefilter sigma in output pixels.
const LENS_PREFILTER_MAX: f32 = 64.0;

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
/// ellipse. Radii under half an output pixel emit no pass. Past twelve output
/// pixels a gaussian of radius/24 first closes the gaps between rings; this
/// stands in for the source's halve-and-upsample levels. Highlight boost is
/// applied to each tap, not to a boosted copy.
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
    let rings = (reach.ceil() as u32).clamp(1, LENS_RINGS);
    let mut passes = if reach > LENS_PREFILTER_FROM {
        sigma_passes(
            (reach / 24.0).min(LENS_PREFILTER_MAX),
            frame.width,
            frame.height,
        )?
    } else {
        Vec::new()
    };
    passes.push(EffectPass::Sample {
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
    });
    Ok(passes)
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
    let pixel = inputs.sample_current(x, y, image_ops::Edge::Clamp);
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

pub(super) fn lens_blur(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let sides = uniform(values, 0) as u32;
    let [rotation, rx, ry, rings, boost, threshold] =
        [1, 2, 3, 4, 5, 6].map(|i| uniform(values, i));
    let rings = rings as u32;
    let (cx, cy) = (x as f32, y as f32);
    let mut sum = lens_tap(inputs, cx, cy, boost, threshold);
    let mut count = 1.0f32;
    for ring in 1..=rings {
        let radius = ring as f32 / rings as f32;
        let n = (ring * 8).max(6);
        for i in 0..n {
            let theta = TAU * (i as f32 + 0.5 * (ring % 2) as f32) / n as f32;
            let reach = radius * iris_radius(sides, theta, rotation);
            let tap = lens_tap(
                inputs,
                cx + theta.cos() * reach * rx,
                cy + theta.sin() * reach * ry,
                boost,
                threshold,
            );
            for (total, value) in sum.iter_mut().zip(tap) {
                *total += value;
            }
            count += 1.0;
        }
    }
    sum.map(|v| v / count)
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
