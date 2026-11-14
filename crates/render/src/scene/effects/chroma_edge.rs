//! The chroma key's two edge stages, Feather edge and Clean up edge. Each is one
//! `Sample` pass behind the keying pass; neither has a shader entry, so the run
//! splitter keeps them on the CPU.
//!
//! The reference exposes only the two 0-100 sliders. What they do is inferred
//! from the controls' names, not measured: Clean up edge contracts the matte
//! (a minimum of coverage over a disc), which drops the thin key-coloured halo,
//! and Feather edge softens it (the mean of coverage over a disc, never above
//! the pixel's own coverage so the removed key colour cannot bleed back).
//! Clean up runs first so the feather softens the cleaned edge. The disc radii
//! at 100 are our scale; the reference's pixel radii are unmeasured.
use super::{
    EffectFrame, EffectGeometry, EffectPass, Error, Result, SampleInputs, Uniform, uniform,
};

/// Shader-style entry names of the two passes.
pub(super) const FEATHER: &str = "matte_feather";
pub(super) const CLEAN: &str = "matte_clean";
/// Disc radius in clip pixels at slider 100.
const FEATHER_CLIP_PX: f64 = 8.0;
const CLEAN_CLIP_PX: f64 = 4.0;
/// Largest disc radius in output pixels.
const RADIUS_MAX: f32 = 64.0;
/// Radii under half an output pixel change nothing and emit no pass.
const RADIUS_MIN: f32 = 0.5;
/// Gather rings of the disc, at most.
const RINGS_MAX: u32 = 6;

/// The edge passes behind a keying pass; empty when both sliders are 0.
pub(super) fn passes(frame: &EffectFrame, geometry: &EffectGeometry) -> Result<Vec<EffectPass>> {
    let range = project::CHROMA_KEY_MIN..=project::CHROMA_KEY_MAX;
    let [gx, gy] = geometry.clip_pixel_scale();
    let scale = gx.min(gy);
    let mut out = Vec::new();
    for (id, entry, clip_px) in [
        ("clean_up_edge", CLEAN, CLEAN_CLIP_PX),
        ("feather_edge", FEATHER, FEATHER_CLIP_PX),
    ] {
        let value = frame.scalar(id)?;
        if !value.is_finite() || !range.contains(&value) {
            return Err(Error::Invalid("chroma key control"));
        }
        let radius = (value / project::CHROMA_KEY_MAX * clip_px * scale) as f32;
        if radius.is_nan() {
            return Err(Error::Invalid("effect spatial extent"));
        }
        let radius = radius.min(RADIUS_MAX);
        if radius >= RADIUS_MIN {
            out.push(EffectPass::Sample {
                entry_point: entry,
                uniforms: vec![Uniform::Float(radius)],
            });
        }
    }
    Ok(out)
}

pub(super) fn valid(values: &[f32], _: u32, _: u32) -> bool {
    (RADIUS_MIN..=RADIUS_MAX).contains(&uniform(values, 0))
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

/// Coverage of the current image at an offset, clamped to the picture edge.
fn coverage(inputs: &SampleInputs<'_>, x: u32, y: u32, dx: i64, dy: i64) -> f32 {
    let at = |v: u32, d: i64, size: u32| {
        (i64::from(v) + d).clamp(0, i64::from(size.max(1)) - 1) as u32
    };
    inputs.texel_current(
        at(x, dx, inputs.current.width()),
        at(y, dy, inputs.current.height()),
    )[3]
}

/// The pixel at a lower coverage; premultiplied colour scales with it.
fn with_coverage(pixel: [f32; 4], coverage: f32) -> [f32; 4] {
    if pixel[3] <= 0.0 {
        return pixel;
    }
    let coverage = coverage.clamp(0.0, pixel[3]);
    let k = coverage / pixel[3];
    [pixel[0] * k, pixel[1] * k, pixel[2] * k, coverage]
}

/// Clean up edge: coverage becomes the minimum over the disc.
pub(super) fn clean(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let pixel = inputs.texel_current(x, y);
    let lowest = taps(uniform(values, 0))
        .map(|(dx, dy)| coverage(inputs, x, y, dx, dy))
        .fold(pixel[3], f32::min);
    with_coverage(pixel, lowest)
}

/// Feather edge: coverage becomes the disc mean, capped at the pixel's own.
pub(super) fn feather(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let pixel = inputs.texel_current(x, y);
    let (sum, count) = taps(uniform(values, 0)).fold((pixel[3], 1.0f32), |(sum, count), (dx, dy)| {
        (sum + coverage(inputs, x, y, dx, dy), count + 1.0)
    });
    with_coverage(pixel, sum / count)
}
