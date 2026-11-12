//! Lights and blurs: light effects that peak at the midpoint while the clips swap underneath.
use super::pixels::{
    PI, add_light, angle_deg, angle_dir, bell, blurred, centre, disc_blur_with, disc_offsets,
    ease_in, ease_out, hash, line_blur, luma, mid_mix, mix, mixed, perceptual_luma, pixel_clamped,
    px, smoothstep, spin_blur, taps, zoom_blur,
};
use super::{Out, Tx};
use crate::{Error, Image, Result};

/// Soft round light blob.
fn blob(x: f32, y: f32, cx: f32, cy: f32, r: f32) -> f32 {
    let d2 = ((x - cx).powi(2) + (y - cy).powi(2)) / (r * r).max(1e-6);
    (-d2).exp()
}

/// The midpoint mix of the outgoing and incoming frames (unwarped).
fn base(t: &Tx, x: usize, y: usize, k: f32) -> [f32; 4] {
    if k <= 0. {
        t.outgoing_at(x, y)
    } else if k >= 1. {
        t.incoming_at(x, y)
    } else {
        mix(t.outgoing_at(x, y), t.incoming_at(x, y), k)
    }
}

pub(super) fn run(id: &str, t: &Tx, out: &mut Out) -> Result<()> {
    let p = t.progress;
    let (w, h) = (t.width as f32, t.height as f32);
    let k = mid_mix(p);
    let env = bell(p);
    match id {
        "colour_burn" => {
            let amount = t.params.fraction("amount")?;
            let hue = p * PI * 2.;
            let tint = [
                1. + 0.5 * hue.cos(),
                1. + 0.5 * (hue + 2.1).cos(),
                1. + 0.5 * (hue + 4.2).cos(),
                1.,
            ];
            out.paint(move |x, y| {
                let c = base(t, x, y, k);
                let g = amount * env * 1.8;
                let l = luma(c);
                [
                    c[0] * (1. + g * tint[0]) + l * g * 0.3,
                    c[1] * (1. + g * tint[1]) + l * g * 0.2,
                    c[2] * (1. + g * tint[2]) + l * g * 0.1,
                    c[3],
                ]
            })
        }
        "light_leak" | "colour_leak" => {
            let amount = t.params.fraction("amount")?;
            let seed = t.params.seed("seed")?;
            let leak = id == "light_leak";
            let user = if leak {
                t.params.color("color")?
            } else {
                [1.; 4]
            };
            let cols: [[f32; 4]; 3] = if leak {
                [
                    user,
                    [user[0], user[1] * 0.8, user[2] * 0.5, 1.],
                    [1., 0.9, 0.7, 1.],
                ]
            } else {
                [[1., 0.15, 0.6, 1.], [0.1, 0.8, 1., 1.], [1., 0.85, 0.1, 1.]]
            };
            let blobs: Vec<(f32, f32, f32, [f32; 4])> = (0..3)
                .map(|i| {
                    let r0 = hash(i, 1, seed);
                    let r1 = hash(i, 2, seed);
                    let side = if leak { 0. } else { r0 };
                    let cx = w * (side + (r1 - 0.5) * 0.4 + (p - 0.5) * (0.8 + r0));
                    let cy = h * (0.2 + 0.6 * hash(i, 3, seed) + (p - 0.5) * (r1 - 0.5));
                    (
                        cx,
                        cy,
                        w.max(h) * (0.25 + 0.25 * hash(i, 4, seed)),
                        match i % 3 {
                            0 => cols[0],
                            1 => cols[1],
                            _ => cols[2],
                        },
                    )
                })
                .collect();
            out.paint(move |x, y| {
                let mut c = base(t, x, y, k);
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                for &(cx, cy, r, colour) in &blobs {
                    c = add_light(c, colour, blob(fx, fy, cx, cy, r) * amount * env * 0.7);
                }
                c
            })
        }
        "zoom_cross" | "zoom_smear" => {
            let (cx, cy) = centre(t)?;
            let strength = t.params.fraction("strength")?;
            let zooms = id == "zoom_cross";
            let za = if zooms {
                1. + 3. * strength * ease_in((p / 0.5).min(1.))
            } else {
                1. + 0.1 * strength * ease_in(p)
            };
            let zb = if zooms {
                1. + 3. * strength * (1. - ease_out(((p - 0.5) / 0.5).clamp(0., 1.)))
            } else {
                1. + 0.1 * strength * (1. - ease_out(p))
            };
            let blur = strength * env * 0.6;
            let n = taps(blur * w.max(h) * 0.5);
            out.paint(move |x, y| {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let s = |image: &Image, zoom: f32| {
                    zoom_blur(
                        image,
                        (fx - cx) / zoom + cx,
                        (fy - cy) / zoom + cy,
                        cx,
                        cy,
                        blur,
                        n,
                    )
                };
                let a = if k < 1. { s(t.outgoing, za) } else { [0.; 4] };
                let b = if k > 0. { s(t.incoming, zb) } else { [0.; 4] };
                mix(a, b, k)
            })
        }
        "streak_smear" => {
            let (dx, dy) = angle_dir(angle_deg(t, "angle")?);
            let length = px(t, "blur")? * env;
            let n = taps(length * 2.);
            let shift = length * 0.5;
            out.paint(move |x, y| {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let a = if k < 1. {
                    line_blur(
                        t.outgoing,
                        fx - dx * shift * p,
                        fy - dy * shift * p,
                        dx * length,
                        dy * length,
                        n,
                    )
                } else {
                    [0.; 4]
                };
                let b = if k > 0. {
                    line_blur(
                        t.incoming,
                        fx + dx * shift * (1. - p),
                        fy + dy * shift * (1. - p),
                        dx * length,
                        dy * length,
                        n,
                    )
                } else {
                    [0.; 4]
                };
                mix(a, b, k)
            })
        }
        "lens_flare" => {
            let col = t.params.color("color")?;
            let amount = t.params.fraction("amount")?;
            let (dx, dy) = angle_dir(angle_deg(t, "angle")? + 90.);
            let diag = (w * w + h * h).sqrt();
            let (fx, fy) = (
                w / 2. + dx * diag * (p - 0.5) * 0.9,
                h / 2. + dy * diag * (p - 0.5) * 0.9,
            );
            let unit = h.max(1.);
            let (gx, gy) = (w - fx, h - fy);
            out.paint(move |x, y| {
                let c = base(t, x, y, k);
                let (px_, py_) = (x as f32 + 0.5, y as f32 + 0.5);
                let (rx, ry) = (px_ - fx, py_ - fy);
                let along = rx * dx + ry * dy;
                let across = -rx * dy + ry * dx;
                let core = blob(px_, py_, fx, fy, unit * 0.06) * 3.
                    + blob(px_, py_, fx, fy, unit * 0.35) * 0.6;
                let streak = (-(across.abs() / (unit * 0.01))).exp()
                    * (-(along.abs() / (diag * 0.35))).exp()
                    * 1.5;
                let ghost = blob(px_, py_, gx, gy, unit * 0.08) * 0.4
                    + blob(px_, py_, (gx + fx) / 2., (gy + fy) / 2., unit * 0.05) * 0.3;
                add_light(c, col, (core + streak + ghost) * amount * env)
            })
        }
        "flash" => {
            let col = t.params.color("color")?;
            let intensity = t.params.fraction("intensity")?.clamp(0., 1.);
            let cut = smoothstep(0.45, 0.55, p);
            let flash = intensity * (-((p - 0.5) / 0.12).powi(2)).exp() * smoothstep(0., 0.2, env);
            out.paint(move |x, y| mix(base(t, x, y, cut), col, flash))
        }
        "bloom" => {
            let r = px(t, "radius")?.max(1.);
            let intensity = t.params.fraction("intensity")?;
            let blended = mixed(t.outgoing, t.incoming, k, out.hooks())?;
            let glow = blurred(&blended, r * 0.5, out.hooks())?;
            let gain = intensity * env * 2.;
            out.paint(move |x, y| {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let c = pixel_clamped(&blended, fx, fy);
                let g = pixel_clamped(&glow, fx, fy);
                [
                    c[0] + g[0] * gain,
                    c[1] + g[1] * gain,
                    c[2] + g[2] * gain,
                    c[3].max((g[3] * gain).min(1.)),
                ]
            })
        }
        "lens_defocus" => {
            let radius = px(t, "radius")?;
            let ra = radius * smoothstep(0., 0.55, p);
            let rb = radius * (1. - smoothstep(0.45, 1., p));
            let n = ((radius * 0.8) as usize).clamp(8, 48);
            let (oa, ob) = (disc_offsets(ra, n), disc_offsets(rb, n));
            out.paint(move |x, y| {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let a = if k < 1. {
                    disc_blur_with(t.outgoing, fx, fy, &oa)
                } else {
                    [0.; 4]
                };
                let b = if k > 0. {
                    disc_blur_with(t.incoming, fx, fy, &ob)
                } else {
                    [0.; 4]
                };
                mix(a, b, k)
            })
        }
        "light_sweep" => {
            let (dx, dy) = angle_dir(angle_deg(t, "angle")?);
            let band = px(t, "width")?.max(2.);
            let col = t.params.color("color")?;
            let amount = t.params.fraction("amount")?;
            let ends =
                [(0., 0.), (w, 0.), (0., h), (w, h)].map(|(cx, cy): (f32, f32)| cx * dx + cy * dy);
            let lo = ends.iter().copied().fold(f32::INFINITY, f32::min);
            let hi = ends.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let edge = lo - band + p * (hi - lo + 2. * band);
            out.paint(move |x, y| {
                let d = (x as f32 + 0.5) * dx + (y as f32 + 0.5) * dy - edge;
                let c = mix(
                    t.outgoing_at(x, y),
                    t.incoming_at(x, y),
                    1. - smoothstep(-band * 0.3, band * 0.3, d),
                );
                add_light(c, col, (-(d / (band * 0.35)).powi(2)).exp() * amount * 2.)
            })
        }
        "phosphor_trail" => {
            let col = t.params.color("color")?;
            let decay = t.params.fraction("decay")?.clamp(0., 1.);
            let kk = smoothstep(0.15, 0.85, p);
            let persist = (1. - p).powf(0.5 + 3. * decay) * smoothstep(0., 0.15, p);
            out.paint(move |x, y| {
                let a = t.outgoing_at(x, y);
                let c = mix(a, t.incoming_at(x, y), kk);
                let l = perceptual_luma(a).powi(2);
                add_light(c, col, l * persist * 1.5)
            })
        }
        "spin_smear" => {
            let (cx, cy) = centre(t)?;
            let max = t.params.num("angle")?.to_radians();
            let ang = max * env;
            let n = taps(ang * w.max(h) * 0.5);
            let (ra, rb) = (max * 0.25 * ease_in(p), -max * 0.25 * (1. - ease_out(p)));
            out.paint(move |x, y| {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let rotate = |r: f32| {
                    let (s, c) = (-r).sin_cos();
                    let (ux, uy) = (fx - cx, fy - cy);
                    (cx + ux * c - uy * s, cy + ux * s + uy * c)
                };
                let a = if k < 1. {
                    let (sx, sy) = rotate(ra);
                    spin_blur(t.outgoing, sx, sy, cx, cy, ang, n)
                } else {
                    [0.; 4]
                };
                let b = if k > 0. {
                    let (sx, sy) = rotate(rb);
                    spin_blur(t.incoming, sx, sy, cx, cy, ang, n)
                } else {
                    [0.; 4]
                };
                mix(a, b, k)
            })
        }
        "light_rays" => {
            let (cx, cy) = centre(t)?;
            let amount = t.params.fraction("amount")?;
            let length = t.params.fraction("length")?.clamp(0., 1.);
            let blended = mixed(t.outgoing, t.incoming, k, out.hooks())?;
            let mut bright = Image::new(blended.width, blended.height, [0.; 4], out.hooks())?;
            let width = blended.width as usize;
            for (row, (output, input)) in bright
                .pixels
                .chunks_mut(width)
                .zip(blended.pixels.chunks(width))
                .enumerate()
            {
                if row % 8 == 0 {
                    crate::check(out.hooks())?;
                }
                for (px, c) in output.iter_mut().zip(input) {
                    let l = (perceptual_luma(*c) - 0.55).max(0.) * 2.2;
                    *px = [c[0] * l, c[1] * l, c[2] * l, c[3] * l];
                }
            }
            let n = 24;
            let gain = amount * env * 3.;
            out.paint(move |x, y| {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let c = pixel_clamped(&blended, fx, fy);
                let r = zoom_blur(&bright, fx, fy, cx, cy, length * 0.9, n);
                [
                    c[0] + r[0] * gain,
                    c[1] + r[1] * gain,
                    c[2] + r[2] * gain,
                    c[3].max((r[3] * gain).min(1.)),
                ]
            })
        }
        "solarise" => {
            let amount = t.params.fraction("amount")?.clamp(0., 1.) * env;
            out.paint(move |x, y| {
                let c = base(t, x, y, k);
                let solar = |v: f32| {
                    let g = v.max(0.).powf(1. / 2.2).min(1.);
                    let s = if g > 0.5 { 1. - g } else { g } * 2.;
                    s.min(1.).powf(2.2)
                };
                let a = c[3];
                let s = [solar(c[0]) * a, solar(c[1]) * a, solar(c[2]) * a, a];
                mix(c, s, amount)
            })
        }
        "stripes" => {
            let n = t.params.num("stripes")?.round().clamp(1., 64.);
            let (dx, dy) = angle_dir(angle_deg(t, "angle")? + 90.);
            let (qx, qy) = (-dy, dx);
            let col = t.params.color("color")?;
            let ends = [(0., 0.), (w, 0.), (0., h), (w, h)];
            let range = |f: &dyn Fn(f32, f32) -> f32| {
                let values = ends.map(|(a, b)| f(a, b));
                (
                    values.iter().copied().fold(f32::INFINITY, f32::min),
                    values.iter().copied().fold(f32::NEG_INFINITY, f32::max),
                )
            };
            let (alo, ahi) = range(&|x, y| x * dx + y * dy);
            let (clo, chi) = range(&|x, y| x * qx + y * qy);
            let span = (ahi - alo).max(1.);
            let bar = span * 0.08;
            out.paint(move |x, y| {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let i = (((fx * qx + fy * qy) - clo) / (chi - clo).max(1.) * n)
                    .floor()
                    .clamp(0., n - 1.);
                let qi = ((p - 0.4 * i / n) / 0.6).clamp(0., 1.);
                let edge = alo - bar + qi * (span + 2. * bar);
                let d = fx * dx + fy * dy - edge;
                let c = mix(
                    t.outgoing_at(x, y),
                    t.incoming_at(x, y),
                    1. - smoothstep(-1., 1., d),
                );
                let light = if d >= 0. && d < bar {
                    (1. - d / bar).powi(2)
                        * smoothstep(0., 0.1, qi)
                        * (1. - smoothstep(0.9, 1., qi))
                } else {
                    0.
                };
                add_light(c, col, light * 1.5)
            })
        }
        _ => Err(Error::Invalid("transition kind")),
    }
}
