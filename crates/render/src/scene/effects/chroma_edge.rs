//! The chroma key's two edge stages, Feather edge and Clean up edge, each behind
//! the keying pass; neither has a shader entry, so the run splitter keeps them on
//! the CPU.
//!
//! Decided from capture round 4 (frames 151402, 151342, 151351, 151406, 151410,
//! 300 % player zoom on a hard polygon edge and a soft ellipse edge, sampled with
//! Pillow):
//! - Clean up edge is spill suppression, not matte contraction. At 0 a bright
//!   green fringe (about #7fcd69 to #9cd590) sits along the soft edge; at 50 and
//!   at 100 every fringe pixel is neutral grey (largest green excess over red
//!   and blue 5 of 255, against 85 at 0) and the two frames are pixel-identical
//!   over the whole player. The fringe keeps its extent, so the matte is
//!   unchanged. The keying pass therefore leaves spill alone and `spill_clean`
//!   removes it: the key's dominant channel loses its excess over the other two,
//!   scaled by `min(value / 50, 1)`. The ramp below 50 is unmeasured.
//! - Feather edge blurs the keyed layer, colour and coverage together, on both
//!   sides of the edge: on the hard polygon edge the 10-90 % transition grows
//!   from a step (0) to about 16 (50) and 50 (100) retina pixels, about 8 and
//!   24 image pixels at the frame's 3x zoom, and the dark side spreads outward
//!   past the original edge. `matte_feather` is the mean of the premultiplied
//!   pixel over a disc (sigma is half the radius), 18 clip pixels at 100, linear
//!   in the slider; the reference's growth is steeper than linear and stays open.
use super::{
    EffectFrame, EffectGeometry, EffectPass, Error, Result, SampleInputs, Uniform, color_uniform,
    uniform,
};

/// Shader-style entry names of the two passes.
pub(super) const FEATHER: &str = "matte_feather";
pub(super) const CLEAN: &str = "spill_clean";
/// Feather disc radius in clip pixels at slider 100.
const FEATHER_CLIP_PX: f64 = 18.0;
/// Clean up edge slider value at which spill is fully removed.
const CLEAN_FULL_AT: f64 = 50.0;
/// Largest disc radius in output pixels.
const RADIUS_MAX: f32 = 64.0;
/// Radii under half an output pixel change nothing and emit no pass.
const RADIUS_MIN: f32 = 0.5;
/// Gather rings of the disc, at most.
const RINGS_MAX: u32 = 6;

/// The edge passes behind a keying pass; empty when both sliders are 0.
pub(super) fn passes(frame: &EffectFrame, geometry: &EffectGeometry) -> Result<Vec<EffectPass>> {
    let range = project::CHROMA_KEY_MIN..=project::CHROMA_KEY_MAX;
    let mut out = Vec::new();
    let read = |id| {
        let value = frame.scalar(id)?;
        if !value.is_finite() || !range.contains(&value) {
            return Err(Error::Invalid("chroma key control"));
        }
        Ok(value)
    };
    let clean = read("clean_up_edge")?;
    let feather = read("feather_edge")?;
    if clean > 0.0 {
        out.push(EffectPass::Pixel {
            entry_point: CLEAN,
            uniforms: vec![
                color_uniform(frame.color("key_color")?),
                Uniform::Float((clean / CLEAN_FULL_AT).min(1.0) as f32),
            ],
        });
    }
    let [gx, gy] = geometry.clip_pixel_scale();
    let radius = (feather / project::CHROMA_KEY_MAX * FEATHER_CLIP_PX * gx.min(gy)) as f32;
    if radius.is_nan() {
        return Err(Error::Invalid("effect spatial extent"));
    }
    let radius = radius.min(RADIUS_MAX);
    if radius >= RADIUS_MIN {
        out.push(EffectPass::Sample {
            entry_point: FEATHER,
            uniforms: vec![Uniform::Float(radius)],
        });
    }
    Ok(out)
}

pub(super) fn feather_valid(values: &[f32], _: u32, _: u32) -> bool {
    values.len() == 1 && (RADIUS_MIN..=RADIUS_MAX).contains(&uniform(values, 0))
}

/// `[key r, key g, key b, strength]`: key in 0..=1, strength in (0, 1].
pub(super) fn clean_valid(values: &[f32]) -> bool {
    let strength = uniform(values, 3);
    values.len() == 4
        && (0..3).all(|i| (0.0..=1.0).contains(&uniform(values, i)))
        && strength > 0.0
        && strength <= 1.0
}

/// Offsets of the disc taps: ring `k` of `n` sits at radius `r*k/n` with
/// `max(8k, 6)` taps, so a radius of one pixel reads its eight neighbours.
fn taps(radius: f32) -> impl Iterator<Item = (i64, i64)> {
    let rings = (radius.ceil() as u32).clamp(1, RINGS_MAX);
    (1..=rings).flat_map(move |ring| {
        let reach = radius * ring as f32 / rings as f32;
        let count = (ring * 8).max(6);
        (0..count).map(move |i| {
            let theta =
                std::f32::consts::TAU * (i as f32 + 0.5 * (ring % 2) as f32) / count as f32;
            (
                (theta.cos() * reach).round() as i64,
                (theta.sin() * reach).round() as i64,
            )
        })
    })
}

/// The current image's premultiplied pixel at an offset, clamped to the picture edge.
fn pixel_at(inputs: &SampleInputs<'_>, x: u32, y: u32, dx: i64, dy: i64) -> [f32; 4] {
    let at = |v: u32, d: i64, size: u32| {
        (i64::from(v) + d).clamp(0, i64::from(size.max(1)) - 1) as u32
    };
    inputs.texel_current(
        at(x, dx, inputs.current.width()),
        at(y, dy, inputs.current.height()),
    )
}

/// Clean up edge: the key's dominant channel (the larger of green and blue in a
/// tie, as the keying pass reads it) loses `strength` of its excess over the
/// larger of the other two, in encoded sRGB on the straight colour.
pub(super) fn spill_pixel(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let [red, green, blue, alpha] = pixel;
    if alpha <= 0.0 {
        return pixel;
    }
    let key = [0, 1, 2].map(|i| uniform(values, i));
    let strength = uniform(values, 3).clamp(0.0, 1.0);
    let [r, g, b] = [red, green, blue].map(|v| crate::image::oetf((v / alpha).clamp(0.0, 1.0), 13));
    let (dominant, limit) = if key[1] >= key[0] && key[1] >= key[2] {
        (1, r.max(b))
    } else if key[2] >= key[0] {
        (2, r.max(g))
    } else {
        (0, g.max(b))
    };
    let mut c = [r, g, b];
    if let Some(channel) = c.get_mut(dominant) {
        *channel -= (*channel - limit).max(0.0) * strength;
    }
    let out = c.map(|v| (crate::image::eotf(v.clamp(0.0, 1.0), 13) * alpha).clamp(0.0, alpha));
    [out[0], out[1], out[2], alpha]
}

/// Feather edge: the pixel becomes the mean of the premultiplied pixels over the disc.
pub(super) fn feather(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let mut sum = inputs.texel_current(x, y);
    let mut count = 1.0f32;
    for (dx, dy) in taps(uniform(values, 0)) {
        let pixel = pixel_at(inputs, x, y, dx, dy);
        for (total, value) in sum.iter_mut().zip(pixel) {
            *total += value;
        }
        count += 1.0;
    }
    let mean = sum.map(|v| v / count);
    let alpha = mean[3].clamp(0.0, 1.0);
    [
        mean[0].clamp(0.0, alpha),
        mean[1].clamp(0.0, alpha),
        mean[2].clamp(0.0, alpha),
        alpha,
    ]
}
