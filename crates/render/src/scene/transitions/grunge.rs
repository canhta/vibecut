//! Grunge and distort: procedural damage that peaks at the midpoint while the clips swap.
use super::pixels::{
    PI, TAU, add_light, bell, fbm, hash, mid_mix, mix, over, sample_clamped, smoothstep, temporal,
    unit, vnoise,
};
use super::wipe::{Style, field_wipe};
use super::{Out, Tx};
use crate::{Error, Result};

/// Per-pixel outgoing/incoming mix at weight `k`, sampled clamped at `(x, y)`.
fn ab(t: &Tx, x: f32, y: f32, k: f32) -> [f32; 4] {
    if k <= 0. {
        sample_clamped(t.outgoing, x, y)
    } else if k >= 1. {
        sample_clamped(t.incoming, x, y)
    } else {
        mix(
            sample_clamped(t.outgoing, x, y),
            sample_clamped(t.incoming, x, y),
            k,
        )
    }
}

/// RGB-split sample: red from `+d`, blue from `-d`.
fn split(t: &Tx, x: f32, y: f32, d: f32, k: f32) -> [f32; 4] {
    let c = ab(t, x, y, k);
    if d.abs() < 0.25 {
        return c;
    }
    let r = ab(t, x + d, y, k);
    let b = ab(t, x - d, y, k);
    [r[0], c[1], b[2], c[3].max(r[3]).max(b[3])]
}

pub(super) fn run(id: &str, t: &Tx, out: &mut Out) -> Result<()> {
    let p = t.progress;
    let (w, h) = (t.width as f32, t.height as f32);
    let env = bell(p);
    let k = mid_mix(p);
    let u = unit(t).max(0.01);
    match id {
        "scramble" => {
            let seed = t.params.seed("seed")?;
            let amount = t.params.fraction("amount")?;
            let frame = (p * 30.).floor() as i32;
            let amp = amount * env * w * 0.15;
            out.paint(move |x, y| {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let (bx, by) = (
                    (fx / (w / 8.)).floor() as i32,
                    (fy / (h / 6.)).floor() as i32,
                );
                let (cx, cy) = (
                    (fx / (w / 23.)).floor() as i32,
                    (fy / (h / 13.)).floor() as i32,
                );
                let r = hash(bx, by, seed.wrapping_add(frame as u32));
                let fine = hash(cx, cy, seed.wrapping_add(frame as u32 * 7 + 1));
                let (ox, oy) = if r > 0.55 {
                    (
                        (hash(bx, by, seed.wrapping_add(99)) - 0.5) * amp,
                        (hash(bx, by, seed.wrapping_add(7)) - 0.5) * amp * 0.3,
                    )
                } else {
                    (0., 0.)
                };
                let swap = fine < k;
                let kk = if swap { 1. } else { 0. };
                let kk = mix([kk; 4], [k; 4], 1. - env)[0];
                split(
                    t,
                    fx - ox,
                    fy - oy,
                    (fine - 0.5) * amount * env * 30. * u,
                    kk,
                )
            })
        }
        "quake" => {
            let seed = t.params.seed("seed")?;
            let shake = super::pixels::px(t, "shake")?;
            let (shutter, n) = super::pixels::shutter(t, 0.03)?;
            let rotation = 0.04 * shake / w.max(1.) * 10.;
            out.paint(move |x, y| {
                temporal(
                    x as f32,
                    y as f32,
                    p,
                    shutter,
                    n,
                    &|x: f32, y: f32, q: f32| {
                        let (fx, fy) = (x + 0.5, y + 0.5);
                        let e = bell(q);
                        let ox = (vnoise(q * 45., 0.5, seed) - 0.5) * 2. * shake * e;
                        let oy =
                            (vnoise(q * 45., 7.5, seed.wrapping_add(1)) - 0.5) * 2. * shake * e;
                        let r = (vnoise(q * 30., 3.5, seed.wrapping_add(2)) - 0.5) * rotation * e;
                        let zoom = 1. + 0.08 * e;
                        let (sn, cs) = r.sin_cos();
                        let (ux, uy) = ((fx - w / 2.) / zoom, (fy - h / 2.) / zoom);
                        let (sx, sy) = (
                            ux * cs - uy * sn + w / 2. - ox,
                            ux * sn + uy * cs + h / 2. - oy,
                        );
                        ab(t, sx, sy, mid_mix(q))
                    },
                )
            })
        }
        "flicker_cut" => {
            let seed = t.params.seed("seed")?;
            let count = t.params.num("flickers")?.round().clamp(1., 40.);
            let j = (p * count * 2.).floor() as i32;
            let show_incoming = hash(j, 3, seed) < smoothstep(0.1, 0.9, p);
            let gain = 1. + (hash(j, 5, seed) - 0.5) * 1.2 * env;
            out.paint(move |x, y| {
                let c = if show_incoming {
                    t.incoming_at(x, y)
                } else {
                    t.outgoing_at(x, y)
                };
                [c[0] * gain, c[1] * gain, c[2] * gain, c[3]]
            })
        }
        "glass_shards" => {
            let seed = t.params.seed("seed")?;
            let cell = super::pixels::px(t, "cell_size")?.max(4.);
            let refraction = super::pixels::px(t, "refraction")?;
            out.paint(move |x, y| {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let (gx, gy) = ((fx / cell).floor() as i32, (fy / cell).floor() as i32);
                let (mut best, mut second, mut id) = (f32::INFINITY, f32::INFINITY, (0, 0));
                for j in -1..=1 {
                    for i in -1..=1 {
                        let (cx, cy) = (gx + i, gy + j);
                        let px_ = (cx as f32 + hash(cx, cy, seed)) * cell;
                        let py_ = (cy as f32 + hash(cx, cy, seed.wrapping_add(1))) * cell;
                        let d = ((fx - px_).powi(2) + (fy - py_).powi(2)).sqrt();
                        if d < best {
                            second = best;
                            best = d;
                            id = (cx, cy);
                        } else if d < second {
                            second = d;
                        }
                    }
                }
                let tau = 0.3 + 0.4 * hash(id.0, id.1, seed.wrapping_add(2));
                let kk = smoothstep(tau - 0.1, tau + 0.1, p);
                let angle = hash(id.0, id.1, seed.wrapping_add(3)) * TAU;
                let offset = refraction * env * hash(id.0, id.1, seed.wrapping_add(4));
                let c = ab(t, fx + angle.cos() * offset, fy + angle.sin() * offset, kk);
                let edge = (-(second - best) / (1.5 * u.max(0.5))).exp() * env;
                let facet = 1. + (hash(id.0, id.1, seed.wrapping_add(5)) - 0.5) * 0.4 * env;
                add_light(
                    [c[0] * facet, c[1] * facet, c[2] * facet, c[3]],
                    [1.; 4],
                    edge * 0.8,
                )
            })
        }
        "digital_glitch" => {
            let seed = t.params.seed("seed")?;
            let amount = t.params.fraction("amount")?;
            let frame = (p * 24.).floor() as u32;
            out.paint(move |x, y| {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let band_h = h / (6. + 18. * hash(frame as i32, 0, seed));
                let band = (fy / band_h.max(1.)).floor() as i32;
                let r = hash(band, frame as i32, seed);
                let ox = if r > 0.6 {
                    (hash(band, frame as i32, seed.wrapping_add(1)) - 0.5) * amount * env * w * 0.25
                } else {
                    0.
                };
                let tau = 0.4 + 0.2 * hash(band, 9, seed);
                let kk = if p > tau { 1. } else { 0. };
                let kk = kk * env + k * (1. - env);
                let c = split(
                    t,
                    fx - ox,
                    fy,
                    amount * env * 18. * u * (r - 0.3).max(0.) * 2.,
                    kk,
                );
                let (bx, by) = (
                    (fx / (24. * u.max(0.2))).floor() as i32,
                    (fy / (12. * u.max(0.2))).floor() as i32,
                );
                if hash(bx, by, seed.wrapping_add(frame * 3)) > 1. - 0.04 * amount * env {
                    let v = hash(bx, by, seed.wrapping_add(77));
                    return mix(c, [v, 1. - v, v * 0.5, 1.], 0.8);
                }
                c
            })
        }
        "rough_edge" => {
            let seed = t.params.seed("seed")?;
            let scale = super::pixels::px(t, "scale")?.max(4.);
            let style = Style::new(
                2. * t.scale,
                super::pixels::px(t, "edge_width")?,
                t.params.color("edge_color")?,
                1.,
            )?;
            field_wipe(
                t,
                style,
                move |x, y| {
                    let noise = fbm(x / scale, y / scale, seed) * 0.8
                        + vnoise(x / (scale * 0.15), y / (scale * 0.15), seed.wrapping_add(9))
                            * 0.2;
                    noise * scale * 3.
                },
                out,
            )
        }
        "kaleidoscope" => {
            let n = t.params.num("segments")?.round().clamp(2., 32.);
            let zoom = 1. + t.params.fraction("zoom")? * env;
            let seg = TAU / n;
            out.paint(move |x, y| {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let (rx, ry) = (fx - w / 2., fy - h / 2.);
                let r = (rx * rx + ry * ry).sqrt() / zoom;
                let th = ry.atan2(rx) + p * PI * 0.5;
                let a = (th.rem_euclid(seg) - seg / 2.).abs();
                let (kx, ky) = (w / 2. + r * a.cos(), h / 2. + r * a.sin());
                let kal = ab(t, kx, ky, k);
                let plain = ab(t, fx, fy, k);
                mix(plain, kal, smoothstep(0., 0.6, env))
            })
        }
        "liquid_warp" => {
            let seed = t.params.seed("seed")?;
            let amount = super::pixels::px(t, "amount")?;
            let scale = super::pixels::px(t, "scale")?.max(4.);
            out.paint(move |x, y| {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let ox = (fbm(fx / scale + p * 2., fy / scale, seed) - 0.5) * 2. * amount * env;
                let oy = (fbm(fx / scale, fy / scale - p * 2., seed.wrapping_add(5)) - 0.5)
                    * 2.
                    * amount
                    * env;
                ab(t, fx + ox, fy + oy, k)
            })
        }
        "tape_damage" => {
            let seed = t.params.seed("seed")?;
            let amount = t.params.fraction("amount")?;
            let frame = (p * 30.).floor() as u32;
            let band_y = h * (1.2 - 1.4 * p);
            out.paint(move |x, y| {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let row = (fy / (2. * u.max(0.5))).floor() as i32;
                let jitter = (hash(row, frame as i32, seed) - 0.5) * amount * env * w * 0.02;
                let in_band = (-((fy - band_y) / (h * 0.06)).powi(2)).exp();
                let ox = jitter
                    + in_band
                        * amount
                        * env
                        * w
                        * 0.08
                        * (vnoise(fy * 0.05, p * 20., seed) - 0.5)
                        * 2.;
                let c = split(t, fx - ox, fy, amount * env * 10. * u, k);
                let grain = (hash(fx as i32, fy as i32, seed.wrapping_add(frame)) - 0.5)
                    * 0.25
                    * amount
                    * env;
                let band_noise = in_band * env * amount * 0.5 * hash(fx as i32 / 3, row, frame);
                let l = super::pixels::luma(c);
                let desat = mix(c, [l, l, l, c[3]], 0.4 * amount * env);
                [
                    desat[0] + grain + band_noise,
                    desat[1] + grain + band_noise,
                    desat[2] + grain + band_noise,
                    desat[3],
                ]
            })
        }
        "screen_off" => {
            let glow = t.params.color("glow_color")?;
            let line = super::pixels::px(t, "line")?.max(1.);
            let (image, q) = if p < 0.5 {
                (t.outgoing, p * 2.)
            } else {
                (t.incoming, (1. - p) * 2.)
            };
            // q: 0 = full picture, 1 = collapsed to a dot.
            let sy = 1. - smoothstep(0., 0.65, q);
            let sx = 1. - smoothstep(0.6, 1., q);
            let hy = (h / 2. * sy).max(line / 2. * sx);
            let hx = w / 2. * sx;
            let white = smoothstep(0.2, 0.7, q);
            out.paint(move |x, y| {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let (rx, ry) = (fx - w / 2., fy - h / 2.);
                // The screen's matte builds with the collapse. An opaque matte
                // at arbitrarily small q would jump from a transparent endpoint.
                let mut c = [0., 0., 0., smoothstep(0., 0.2, q)];
                if rx.abs() <= hx && ry.abs() <= hy && hx > 0. {
                    let lx = w / 2. + rx / sx.max(1e-3);
                    let ly = h / 2. + ry / sy.max(1e-3);
                    let src = sample_clamped(image, lx, ly.clamp(0., h));
                    c = over(c, mix(src, glow, white));
                }
                let dy = (ry.abs() - hy).max(0.);
                let dx = (rx.abs() - hx).max(0.);
                let halo = (-(dx * dx + dy * dy).sqrt() / (line * 3.)).exp() * white;
                add_light(c, glow, halo * 0.8)
            })
        }
        _ => Err(Error::Invalid("transition kind")),
    }
}
