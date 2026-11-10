//! Dissolves: additive light, filmic gamma, soft focus, burn, brightness and pixel blocks.
use super::pixels::{
    add_light, bell, blurred, fbm, hash, mid_mix, mix, perceptual_luma, px, sample_clamped,
    smoothstep, unit,
};
use super::{Out, Tx};
use crate::Result;

pub(super) fn run(id: &str, t: &Tx, out: &mut Out) -> Result<()> {
    let p = t.progress;
    match id {
        "light_fade" => {
            let (ka, kb) = ((2. * (1. - p)).min(1.), (2. * p).min(1.));
            out.paint(move |x, y| {
                std::array::from_fn(|i| {
                    let (a, b) = (
                        t.outgoing_at(x, y).get(i).copied().unwrap_or(0.),
                        t.incoming_at(x, y).get(i).copied().unwrap_or(0.),
                    );
                    (a * ka + b * kb).min(1.)
                })
            })
        }
        "film_fade" => out.paint(move |x, y| {
            let a = t.outgoing_at(x, y);
            let b = t.incoming_at(x, y);
            std::array::from_fn(|i| {
                let gamma = 1. / 2.2;
                let v = a.get(i).copied().unwrap_or(0.).max(0.).powf(gamma) * (1. - p)
                    + b.get(i).copied().unwrap_or(0.).max(0.).powf(gamma) * p;
                v.powf(2.2)
            })
        }),
        "soft_focus_fade" => {
            let sigma = px(t, "blur")? * 0.5;
            let k = smoothstep(0.25, 0.75, p);
            let sa = sigma * smoothstep(0., 0.6, p);
            let sb = sigma * (1. - smoothstep(0.4, 1., p));
            let ba = blurred(t.outgoing, sa, out.hooks())?;
            let bb = blurred(t.incoming, sb, out.hooks())?;
            out.paint(move |x, y| {
                mix(
                    sample_clamped(&ba, x as f32 + 0.5, y as f32 + 0.5),
                    sample_clamped(&bb, x as f32 + 0.5, y as f32 + 0.5),
                    k,
                )
            })
        }
        "burn_through" => {
            let col = t.params.color("burn_color")?;
            let soft = t.params.fraction("softness")?.clamp(0.01, 1.) * 0.3;
            let seed = t.params.seed("seed")?;
            let scale = (220. * unit(t)).max(1.);
            let thr = -soft + p * (1. + 2. * soft);
            let fade = (p * 10.).min((1. - p) * 10.).min(1.);
            out.paint(move |x, y| {
                let a = t.outgoing_at(x, y);
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                // Burn order: dark areas of the outgoing frame and the noise go first.
                let g = 0.55 * fbm(fx / scale, fy / scale, seed) + 0.45 * (1. - perceptual_luma(a));
                let d = g - thr;
                let k = 1. - smoothstep(-soft * 0.3, soft * 0.3, d);
                let base = mix(a, t.incoming_at(x, y), k);
                let edge = (-(d / (soft * 0.6)).powi(2)).exp() * fade;
                let charred = if d > 0. {
                    1. - 0.6 * (-(d / soft).powi(2)).exp() * fade
                } else {
                    1.
                };
                add_light(
                    [
                        base[0] * charred,
                        base[1] * charred,
                        base[2] * charred,
                        base[3],
                    ],
                    col,
                    edge * 2.,
                )
            })
        }
        "brightness_fade" => {
            let soft = t.params.fraction("softness")?.clamp(0.005, 1.) * 0.5;
            let from_b = t.params.choice("source")? == 1;
            let invert = t.params.flag("invert")?;
            let thr = -soft + p * (1. + 2. * soft);
            out.paint(move |x, y| {
                let (a, b) = (t.outgoing_at(x, y), t.incoming_at(x, y));
                let l = perceptual_luma(if from_b { b } else { a });
                let g = if invert { l } else { 1. - l };
                mix(a, b, smoothstep(-soft, soft, thr - g))
            })
        }
        "pixel_blocks" => {
            let max = px(t, "block_size")?.max(1.);
            let bs = 1. + (max - 1.) * bell(p);
            let k = mid_mix(p);
            let per_block = t.params.flag("dissolve")?;
            out.paint(move |x, y| {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                if bs < 1.5 {
                    return mix(t.outgoing_at(x, y), t.incoming_at(x, y), k);
                }
                let (bx, by) = ((fx / bs).floor(), (fy / bs).floor());
                let (sx, sy) = ((bx + 0.5) * bs, (by + 0.5) * bs);
                let kk = if per_block {
                    let r = hash(bx as i32, by as i32, 7);
                    smoothstep(r - 0.15, r + 0.15, k * 1.3 - 0.15)
                } else {
                    k
                };
                mix(
                    sample_clamped(t.outgoing, sx, sy),
                    sample_clamped(t.incoming, sx, sy),
                    kk,
                )
            })
        }
        _ => Err(crate::Error::Invalid("transition kind")),
    }
}
