//! Per-item noise reduction: one edge-preserving bilateral sample pass.
//!
//! The pass reads only the image it is given (no temporal history, no analysis
//! job), so a frame's result depends on that frame alone and preview equals
//! export by construction. It runs before the item's colour effects.
use super::{EffectPass, SampleInputs, Uniform, encoded_color, straight};

/// Window half-width in output pixels: a 5x5 window.
const RADIUS: i32 = 2;
/// Spatial gaussian sigma in output pixels.
const SIGMA_SPATIAL: f32 = 1.2;
/// Range gaussian sigma on straight sRGB-encoded colour and alpha, `0..=1` scale.
const SIGMA_RANGE: f32 = 0.1;

/// The single noise-reduction pass. Strength is one fixed level: the reference
/// shows an on/off toggle only.
pub fn reduce_noise_passes() -> Vec<EffectPass> {
    vec![EffectPass::Sample {
        entry_point: "reduce_noise",
        uniforms: vec![Uniform::Float(SIGMA_SPATIAL), Uniform::Float(SIGMA_RANGE)],
    }]
}

/// Uniforms are the two sigmas, finite and positive.
pub(super) fn valid_reduce_noise(values: &[f32], _: u32, _: u32) -> bool {
    values
        .iter()
        .all(|sigma| sigma.is_finite() && (0.05..=8.0).contains(sigma))
}

/// Bilateral average of the current image over the 5x5 window. A tap's weight is
/// the spatial gaussian times a range gaussian of its encoded-colour and alpha
/// distance to the centre, so taps across an edge weigh almost nothing. Taps
/// outside the image are skipped and the weights renormalise. The centre weighs
/// one, so the sum is never zero, and the result is a convex combination of
/// premultiplied pixels and keeps the working-image contract.
pub(super) fn reduce_noise_sample(
    inputs: &SampleInputs<'_>,
    x: u32,
    y: u32,
    values: &[f32],
) -> [f32; 4] {
    let Some(&[sigma_spatial, sigma_range]) = values.first_chunk::<2>() else {
        return inputs.texel_current(x, y);
    };
    let centre = inputs.texel_current(x, y);
    let (centre_colour, centre_alpha) = straight(centre);
    let centre_colour = encoded_color(centre_colour);
    let spatial = -0.5 / (sigma_spatial * sigma_spatial);
    let range = -0.5 / (sigma_range * sigma_range);
    let (width, height) = (inputs.current.width, inputs.current.height);
    let mut sum = [0.0_f32; 4];
    let mut total = 0.0_f32;
    for dy in -RADIUS..=RADIUS {
        let Some(ty) = y.checked_add_signed(dy).filter(|ty| *ty < height) else {
            continue;
        };
        for dx in -RADIUS..=RADIUS {
            let Some(tx) = x.checked_add_signed(dx).filter(|tx| *tx < width) else {
                continue;
            };
            let tap = inputs.texel_current(tx, ty);
            let (colour, alpha) = straight(tap);
            let colour = encoded_color(colour);
            let mut distance = (alpha - centre_alpha) * (alpha - centre_alpha);
            for (a, b) in colour.iter().zip(centre_colour) {
                distance += (a - b) * (a - b);
            }
            let weight = ((dx * dx + dy * dy) as f32 * spatial + distance * range).exp();
            for (out, value) in sum.iter_mut().zip(tap) {
                *out += weight * value;
            }
            total += weight;
        }
    }
    if total > 0.0 {
        sum.map(|v| v / total)
    } else {
        centre
    }
}
