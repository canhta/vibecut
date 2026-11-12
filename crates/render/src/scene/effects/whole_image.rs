//! CPU references of the 265 whole-image entries: coordinate warps, glows,
//! stylize kernels, generators and keys.
//!
//! Every reference reads one output pixel from its coordinate, flat uniforms and
//! the two readable images. Multi-stage entries compose the existing `Pixel`,
//! `Convolve` and `Sample` passes instead of introducing a new pass kind. The
//! `fine` flag may lower sample counts only; every entry keeps its geometry,
//! colour mapping and affected pixels in both qualities.
use super::{
    EffectFrame, EffectGeometry, EffectPass, Error, Result, SampleInputs, Uniform, decoded_color,
    encoded_color, image_ops, luma, premultiply, smoothstep, straight, texel,
};
use std::f32::consts::{FRAC_PI_2, TAU};
use time::Rounding;

fn uniform(values: &[f32], index: usize) -> f32 {
    values.get(index).copied().unwrap_or(0.0)
}
fn color_at(values: &[f32], start: usize) -> [f32; 3] {
    [0, 1, 2].map(|k| uniform(values, start + k))
}
fn point_at(values: &[f32], start: usize) -> [f32; 2] {
    [uniform(values, start), uniform(values, start + 1)]
}
fn finite(values: &[f32], len: usize) -> bool {
    values.len() == len && values.iter().all(|v| v.is_finite())
}

// ------------------------------------------------------------------ noise

fn hash_u(x: i64, y: i64, z: i64, seed: u64) -> u64 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (z as u64).wrapping_mul(0x1656_67B1_9E37_79F9)
        ^ seed.wrapping_mul(0x27D4_EB2F_1656_67C5);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^ (h >> 32)
}
fn hash01(x: i64, y: i64, z: i64, seed: u64) -> f32 {
    (hash_u(x, y, z, seed) >> 40) as f32 / (1u64 << 24) as f32
}
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}
fn grad3(h: u64, x: f32, y: f32, z: f32) -> f32 {
    match h % 12 {
        0 => x + y,
        1 => -x + y,
        2 => x - y,
        3 => -x - y,
        4 => x + z,
        5 => -x + z,
        6 => x - z,
        7 => -x - z,
        8 => y + z,
        9 => -y + z,
        10 => y - z,
        _ => -y - z,
    }
}
fn noise3(x: f32, y: f32, z: f32, seed: u64) -> f32 {
    let (xi, yi, zi) = (x.floor(), y.floor(), z.floor());
    let (xf, yf, zf) = (x - xi, y - yi, z - zi);
    let (xi, yi, zi) = (xi as i64, yi as i64, zi as i64);
    let (u, v, w) = (fade(xf), fade(yf), fade(zf));
    let g = |dx: i64, dy: i64, dz: i64| {
        grad3(
            hash_u(xi + dx, yi + dy, zi + dz, seed),
            xf - dx as f32,
            yf - dy as f32,
            zf - dz as f32,
        )
    };
    let l = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = l(g(0, 0, 0), g(1, 0, 0), u);
    let x10 = l(g(0, 1, 0), g(1, 1, 0), u);
    let x01 = l(g(0, 0, 1), g(1, 0, 1), u);
    let x11 = l(g(0, 1, 1), g(1, 1, 1), u);
    l(l(x00, x10, v), l(x01, x11, v), w) * 0.9
}
fn fbm(x: f32, y: f32, z: f32, octaves: f32, seed: u64) -> f32 {
    let n = octaves.clamp(1.0, 10.0);
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    let mut i = 0;
    while (i as f32) < n {
        let k = (n - i as f32).min(1.0);
        sum += noise3(
            x * freq,
            y * freq,
            z * freq + i as f32 * 17.13,
            seed.wrapping_add(i as u64),
        ) * amp
            * k;
        norm += amp * k;
        amp *= 0.5;
        freq *= 2.0;
        i += 1;
    }
    sum / norm.max(1e-6)
}
fn noise1(t: f64, channel: u64, seed: u64) -> f32 {
    noise3(t as f32, channel as f32 * 7.31 + 0.5, 0.37, seed)
}
fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    smoothstep(e0, e1.max(e0 + 1e-6), x)
}

// ------------------------------------------------------------------ warps

pub(super) fn twirl(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let [cx, cy, angle, radius] = [0, 1, 2, 3].map(|i| uniform(values, i));
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let (dx, dy) = (px - cx, py - cy);
    let d = dx.hypot(dy);
    if radius <= 0.0 || d >= radius {
        return inputs.texel_current(x, y);
    }
    let k = 1.0 - d / radius;
    let a = angle * k * k;
    let (sin, cos) = a.sin_cos();
    inputs.sample_current(
        cx + dx * cos - dy * sin - 0.5,
        cy + dx * sin + dy * cos - 0.5,
        image_ops::Edge::Clamp,
    )
}

pub(super) fn spherize(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let [cx, cy, radius] = [0, 1, 2].map(|i| uniform(values, i));
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let (dx, dy) = (px - cx, py - cy);
    let d = dx.hypot(dy);
    if radius < 0.5 || d >= radius || d < 1e-9 {
        return inputs.texel_current(x, y);
    }
    let n = d / radius;
    let k = 1.0 - 0.5 * (1.0 - n * n).max(0.0).sqrt();
    inputs.sample_current(cx + dx * k - 0.5, cy + dy * k - 0.5, image_ops::Edge::Clamp)
}

pub(super) fn magnify(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let [cx, cy, magnification, size, feather, shape] =
        [0, 1, 2, 3, 4, 5].map(|i| uniform(values, i));
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let (dx, dy) = (px - cx, py - cy);
    let d = if shape > 0.5 {
        dx.abs().max(dy.abs())
    } else {
        dx.hypot(dy)
    };
    let k = if feather > 0.0 {
        ((size - d) / feather).clamp(0.0, 1.0)
    } else {
        (size - d + 0.5).clamp(0.0, 1.0)
    };
    if k <= 0.0 {
        return inputs.texel_current(x, y);
    }
    let m = (magnification / 100.0).max(0.01);
    let sample =
        inputs.sample_current(cx + dx / m - 0.5, cy + dy / m - 0.5, image_ops::Edge::Clamp);
    let base = inputs.texel_current(x, y);
    std::array::from_fn(|channel| base[channel] + (sample[channel] - base[channel]) * k)
}

pub(super) fn wave_warp(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let [height, width, direction, phase] = [0, 1, 2, 3].map(|i| uniform(values, i));
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let (ux, uy) = (direction.sin(), -direction.cos());
    let along = px * ux + py * uy;
    let offset = height * (along / width.max(1e-3) * TAU - phase).sin();
    inputs.sample_current(
        px - uy * offset - 0.5,
        py + ux * offset - 0.5,
        image_ops::Edge::Clamp,
    )
}

pub(super) fn lens_distortion(
    inputs: &SampleInputs<'_>,
    x: u32,
    y: u32,
    values: &[f32],
) -> [f32; 4] {
    let [k, cx, cy, norm] = [0, 1, 2, 3].map(|i| uniform(values, i));
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let (dx, dy) = ((px - cx) / norm, (py - cy) / norm);
    let scale = 1.0 - k * (dx * dx + dy * dy);
    inputs.sample_current(
        cx + dx * scale * norm - 0.5,
        cy + dy * scale * norm - 0.5,
        image_ops::Edge::Clamp,
    )
}

pub(super) fn turbulent_displace(
    inputs: &SampleInputs<'_>,
    x: u32,
    y: u32,
    values: &[f32],
) -> [f32; 4] {
    let [kind, amount, size, ox, oy, complexity, evolution, coarse] =
        [0, 1, 2, 3, 4, 5, 6, 7].map(|i| uniform(values, i));
    let kind = kind.round() as i32;
    let complexity = if coarse > 0.5 {
        complexity.min(3.0)
    } else {
        complexity
    };
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let size = size.max(0.5);
    let (u, v) = (px / size + ox, py / size + oy);
    let n = |x: f32, y: f32, channel: f32| {
        fbm(
            x + channel * 31.7,
            y,
            evolution + channel * 3.1,
            complexity,
            0,
        )
    };
    let (dx, dy) = match kind {
        0 | 3 => (n(u, v, 0.0), n(u, v, 1.0)),
        1 | 4 | 2 | 5 => {
            let eps = 0.05;
            let gx = (n(u + eps, v, 0.0) - n(u - eps, v, 0.0)) / (2.0 * eps);
            let gy = (n(u, v + eps, 0.0) - n(u, v - eps, 0.0)) / (2.0 * eps);
            if kind == 1 || kind == 4 {
                (gx * 0.3, gy * 0.3)
            } else {
                (-gy * 0.3, gx * 0.3)
            }
        }
        6 => (0.0, n(u, v, 0.0)),
        7 => (n(u, v, 0.0), 0.0),
        _ => {
            let q = n(u, v, 0.0);
            (q, q)
        }
    };
    inputs.sample_current(
        px - dx * amount - 0.5,
        py - dy * amount - 0.5,
        image_ops::Edge::Clamp,
    )
}

pub(super) fn corner_pin(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let m = [0, 1, 2, 3, 4, 5, 6, 7, 8].map(|i| uniform(values, i));
    let [width, height] = [9, 10].map(|i| uniform(values, i));
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let w = m[6] * px + m[7] * py + m[8];
    if w.abs() < 1e-12 {
        return [0.0; 4];
    }
    let u = (m[0] * px + m[1] * py + m[2]) / w;
    let v = (m[3] * px + m[4] * py + m[5]) / w;
    if !(-1e-3..=1.0 + 1e-3).contains(&u) || !(-1e-3..=1.0 + 1e-3).contains(&v) {
        return [0.0; 4];
    }
    inputs.sample_input(
        u * width - 0.5,
        v * height - 0.5,
        image_ops::Edge::Transparent,
    )
}

pub(super) fn basic_3d(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let [swivel, tilt, distance, focal, width, height] =
        [0, 1, 2, 3, 4, 5].map(|i| uniform(values, i));
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let (px, py) = (px - width / 2.0, py - height / 2.0);
    let (ss, cs) = swivel.sin_cos();
    let (st, ct) = tilt.sin_cos();
    let n = [ss * ct, -st, cs * ct];
    let u = [cs, 0.0, -ss];
    let v = [ss * st, ct, cs * st];
    let z0 = focal + distance * 10.0;
    let dir = [px, py, focal];
    let denom = n[0] * dir[0] + n[1] * dir[1] + n[2] * dir[2];
    if denom.abs() < 1e-9 {
        return [0.0; 4];
    }
    let t = (n[2] * z0) / denom;
    if t <= 0.0 {
        return [0.0; 4];
    }
    let hit = [dir[0] * t, dir[1] * t, dir[2] * t - z0];
    let su = hit[0] * u[0] + hit[1] * u[1] + hit[2] * u[2];
    let sv = hit[0] * v[0] + hit[1] * v[1] + hit[2] * v[2];
    inputs.sample_input(
        su + width / 2.0 - 0.5,
        sv + height / 2.0 - 0.5,
        image_ops::Edge::Transparent,
    )
}

// ------------------------------------------------------------------ stylize

pub(super) fn brush_strokes(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let [angle, size, length, cell, steps] = [0, 1, 2, 3, 4].map(|i| uniform(values, i));
    let steps = (steps.round() as u32).clamp(2, 24);
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let (cxi, cyi) = ((px / cell).floor() as i64, (py / cell).floor() as i64);
    let jitter = (hash01(cxi, cyi, 1, 77) - 0.5) * 0.5 * FRAC_PI_2;
    let a = angle + jitter;
    let (dx, dy) = (a.cos(), -a.sin());
    let (nx, ny) = (-dy, dx);
    let mut acc = [0.0f32; 4];
    let mut count = 0.0_f32;
    for i in 0..steps {
        let t = (i as f32 / (steps - 1) as f32 - 0.5) * length;
        for s in [-0.5f32, 0.0, 0.5] {
            let q = inputs.sample_input(
                px + dx * t + nx * s * size - 0.5,
                py + dy * t + ny * s * size - 0.5,
                image_ops::Edge::Clamp,
            );
            for (out, value) in acc.iter_mut().zip(q) {
                *out += value;
            }
            count += 1.0;
        }
    }
    let mut out = acc.map(|v| v / count.max(1.0));
    let perpendicular = px * nx + py * ny;
    let bristle = 1.0
        + 0.1
            * noise3(
                perpendicular / size.max(1.0),
                (px * dx + py * dy) / length.max(1.0),
                cxi as f32 * 0.37,
                5,
            );
    for v in &mut out[..3] {
        *v *= bristle;
    }
    let coverage = (0.75 + 0.25 * hash01(cxi, cyi, 3, 77)).min(1.0);
    out.map(|v| v * coverage)
}

pub(super) fn roughen_edges(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let kind = uniform(values, 0).round() as i32;
    let edge = color_at(values, 1);
    let [_border, scale, complexity, coarse] = [4, 5, 6, 7].map(|i| uniform(values, i));
    let complexity = if coarse > 0.5 {
        complexity.min(3.0)
    } else {
        complexity
    };
    let centre = inputs.texel_input(x, y);
    if centre[3] <= 1e-6 {
        return centre;
    }
    let depth = inputs.texel_current(x, y);
    let d = depth[0].clamp(0.0, 1.0);
    let (u, v) = (x as f32 / scale.max(0.5), y as f32 / scale.max(0.5));
    let mut n = fbm(u, v, 0.0, complexity, 0);
    if kind == 3 {
        n = 1.0 - 2.0 * n.abs();
    }
    let sharpness = 20.0;
    let mut k = ((d - 0.5 + n * 0.5) * (1.0 + sharpness) + 0.5).clamp(0.0, 1.0);
    if kind == 2 {
        k = if k > 0.5 { 1.0 } else { 0.0 };
    }
    if (4..=7).contains(&kind) {
        let hole = fbm(u * 2.3 + 11.0, v * 2.3, 0.0, complexity, 1);
        if d < 0.97 && hole > 0.25 {
            k *= 1.0 - smooth(0.25, 0.4, hole) * (1.0 - d);
        }
    }
    let mut out = centre;
    if matches!(kind, 1 | 5 | 7) && d < 0.97 {
        let t = ((1.0 - d) * 1.5).min(1.0);
        let (c, alpha) = straight(centre);
        let mixed = [0, 1, 2].map(|i| c[i] + (edge[i] - c[i]) * t);
        out = premultiply(mixed, alpha);
    }
    if matches!(kind, 6 | 7) {
        let (c, alpha) = straight(out);
        let level = if crate::image::oetf(luma(c).max(0.0), 13) > 0.5 {
            1.0
        } else {
            0.0
        };
        out = premultiply([level; 3], alpha);
    }
    out.map(|v| v * k)
}

pub(super) fn camera_shake(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let [amount, rotation, zoom, frequency, seconds] = [0, 1, 2, 3, 4].map(|i| uniform(values, i));
    let shake = |t: f64, channel: u64| -> f32 {
        let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
        for octave in 0..3u64 {
            sum += noise1(
                t * f64::from(frequency) * f64::from(freq),
                channel * 10 + octave,
                0,
            ) * amp;
            norm += amp;
            amp *= 0.5;
            freq *= 2.0;
        }
        sum / norm.max(1e-6) * 1.6
    };
    let t = f64::from(seconds);
    let offset = [shake(t, 0) * amount, shake(t, 1) * amount];
    let scale = 1.0 + zoom * (0.5 + 0.5 * shake(t, 3));
    let angle = shake(t, 2) * rotation;
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let (w, h) = (inputs.input.width() as f32, inputs.input.height() as f32);
    let (qx, qy) = (
        (px - w / 2.0 - offset[0]) / scale,
        (py - h / 2.0 - offset[1]) / scale,
    );
    let (sin, cos) = angle.sin_cos();
    let (rx, ry) = (qx * cos - qy * sin, qx * sin + qy * cos);
    inputs.sample_input(
        rx + w / 2.0 - 0.5,
        ry + h / 2.0 - 0.5,
        image_ops::Edge::Clamp,
    )
}

// ------------------------------------------------------------------ glows

pub(super) fn highlights(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let alpha = pixel[3];
    if alpha <= 1e-6 {
        return [0.0; 4];
    }
    let threshold = uniform(values, 0).clamp(0.0, 1.0);
    let soft = uniform(values, 1).max(1e-3);
    let (colour, _) = straight(pixel);
    let level = luma(encoded_color(colour));
    let k = smooth(threshold, threshold + soft, level);
    pixel.map(|v| v * k)
}

pub(super) fn alpha_plane(pixel: [f32; 4], _: &[f32]) -> [f32; 4] {
    let alpha = pixel[3];
    [alpha, alpha, alpha, 1.0]
}

pub(super) fn sobel_edges(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let threshold = uniform(values, 0).clamp(0.0, 1.0);
    let (w, h) = (
        inputs.current.width() as i64,
        inputs.current.height() as i64,
    );
    let l = |dx: i64, dy: i64| {
        let sx = (x as i64 + dx).clamp(0, w - 1) as u32;
        let sy = (y as i64 + dy).clamp(0, h - 1) as u32;
        let p = inputs.texel_current(sx, sy);
        crate::image::oetf(luma([p[0], p[1], p[2]]).clamp(0.0, 1.0), 13) * p[3].min(1.0)
    };
    let gx = l(1, -1) + 2.0 * l(1, 0) + l(1, 1) - l(-1, -1) - 2.0 * l(-1, 0) - l(-1, 1);
    let gy = l(-1, 1) + 2.0 * l(0, 1) + l(1, 1) - l(-1, -1) - 2.0 * l(0, -1) - l(1, -1);
    let level = smooth(threshold, threshold + 0.1, gx.hypot(gy) / 4.0);
    [level; 4]
}

pub(super) fn glow_add(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let [r, g, b, intensity, saturation, tint, screen, only] =
        [0, 1, 2, 3, 4, 5, 6, 7].map(|i| uniform(values, i));
    let glow = inputs.texel_current(x, y);
    let mut colour = [glow[0], glow[1], glow[2]];
    if tint > 0.5 {
        let l = luma(colour).max(0.0);
        colour = [l * r * 1.5, l * g * 1.5, l * b * 1.5];
    } else if (saturation - 1.0).abs() > 1e-4 {
        let l = luma(colour);
        colour = colour.map(|v| l + (v - l) * saturation);
    }
    let light = colour.map(|v| (v * intensity).max(0.0));
    if only > 0.5 {
        let alpha = light[0].max(light[1]).max(light[2]).min(1.0);
        return [light[0], light[1], light[2], alpha];
    }
    let base = inputs.texel_input(x, y);
    let mut out = base;
    if screen > 0.5 {
        for k in 0..3 {
            let add = light[k].min(1.0);
            out[k] = (base[k] + add - base[k].min(1.0) * add).max(0.0);
        }
        out[3] = base[3] + (1.0 - base[3]) * light[0].max(light[1]).max(light[2]).min(1.0);
    } else {
        for k in 0..3 {
            out[k] = (base[k] + light[k]).max(0.0);
        }
        out[3] = base[3] + (1.0 - base[3]) * light[0].max(light[1]).max(light[2]).min(1.0);
    }
    out
}

pub(super) fn glint_rays(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let base = inputs.texel_input(x, y);
    let colour = color_at(values, 0);
    let [intensity, length, rotation, rays, colorize, steps] =
        [3, 4, 5, 6, 7, 8].map(|i| uniform(values, i));
    let rays = (rays.round() as u32).clamp(2, 8);
    let steps = (steps.round() as u32).clamp(2, 64);
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let mut total = [0.0f32; 4];
    for ray in 0..rays {
        let a = rotation + TAU * ray as f32 / rays as f32;
        let (dx, dy) = (a.cos() * length * 0.35, a.sin() * length * 0.35);
        let decay = (-1.0f32 / length.max(1.0)).exp();
        let weight_sum = if (1.0 - decay).abs() < 1e-6 {
            steps as f32
        } else {
            decay * (1.0 - decay.powi(steps as i32)) / (1.0 - decay)
        };
        let weight_sum = weight_sum.max(1e-6);
        let mut weight = 1.0;
        for step in 1..=steps {
            weight *= decay;
            let t = step as f32 * length / steps as f32;
            let q =
                inputs.sample_current(px - dx * t - 0.5, py - dy * t - 0.5, image_ops::Edge::Clamp);
            for (out, value) in total.iter_mut().zip(q) {
                *out += value * weight / weight_sum;
            }
        }
    }
    total = total.map(|v| v / rays as f32);
    if colorize > 0.0 {
        let l = luma([total[0], total[1], total[2]]).max(0.0);
        let tinted = [
            l * colour[0] * 1.5,
            l * colour[1] * 1.5,
            l * colour[2] * 1.5,
        ];
        for k in 0..3 {
            total[k] += (tinted[k] - total[k]) * colorize.clamp(0.0, 1.0);
        }
    }
    let light = [
        total[0] * intensity,
        total[1] * intensity,
        total[2] * intensity,
    ];
    let mut out = base;
    for k in 0..3 {
        out[k] = (base[k] + light[k]).max(0.0);
    }
    out[3] = base[3] + (1.0 - base[3]) * light[0].max(light[1]).max(light[2]).min(1.0);
    out
}

pub(super) fn radial_rays(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let [cx, cy, length, intensity, r, g, b, only, steps] =
        [0, 1, 2, 3, 4, 5, 6, 7, 8].map(|i| uniform(values, i));
    let steps = (steps.round() as u32).clamp(4, 64);
    let base = inputs.texel_input(x, y);
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let mut acc = [0.0f32; 4];
    let mut weight_sum = 0.0;
    for i in 0..steps {
        let t = i as f32 / steps as f32 * length;
        let q = inputs.sample_current(
            px + (cx - px) * t - 0.5,
            py + (cy - py) * t - 0.5,
            image_ops::Edge::Clamp,
        );
        let weight = 1.0 - i as f32 / steps as f32 * 0.6;
        for (out, value) in acc.iter_mut().zip(q) {
            *out += value * weight;
        }
        weight_sum += weight;
    }
    let glow = acc.map(|v| v / weight_sum.max(1e-6));
    let light = [
        glow[0] * r * intensity * 2.5,
        glow[1] * g * intensity * 2.5,
        glow[2] * b * intensity * 2.5,
    ];
    if only > 0.5 {
        let alpha = light[0].max(light[1]).max(light[2]).min(1.0);
        return [light[0], light[1], light[2], alpha];
    }
    let mut out = base;
    for k in 0..3 {
        out[k] = (base[k] + light[k]).max(0.0);
    }
    out[3] = base[3] + (1.0 - base[3]) * light[0].max(light[1]).max(light[2]).min(1.0);
    out
}

pub(super) fn light_leaks(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let c1 = color_at(values, 0);
    let c2 = color_at(values, 3);
    let [intensity, scale, direction, phase, seed, mode] =
        [6, 7, 8, 9, 10, 11].map(|i| uniform(values, i));
    let base = inputs.texel_input(x, y);
    let (w, h) = (inputs.input.width() as f32, inputs.input.height() as f32);
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let (dx, dy) = (direction.cos(), direction.sin());
    let along = ((px / w - 0.5) * dx + (py / h - 0.5) * dy) * 2.0;
    let bias = smooth(-0.2, 1.0, -along);
    let (u, v) = (px / scale, py / scale);
    let seed = seed.round().max(0.0) as u64;
    let n = fbm(
        u + dx * phase * 0.3,
        v + dy * phase * 0.3,
        phase * 0.15,
        3.0,
        seed,
    ) * 0.5
        + 0.5;
    let m = fbm(u * 0.7 + 5.2, v * 0.7, phase * 0.1 + 9.0, 2.0, seed) * 0.5 + 0.5;
    let k = smooth(0.35, 0.85, n) * bias * intensity;
    let light = [0, 1, 2].map(|i| c1[i] + (c2[i] - c1[i]) * m);
    let light = light.map(|v| v * k);
    let (colour, alpha) = straight(base);
    let blended = match mode.round() as u32 {
        1 => [0, 1, 2].map(|i| colour[i] + light[i]),
        2 => [0, 1, 2].map(|i| {
            let (b, s) = (colour[i], (colour[i] + light[i] + 0.5).min(1.0));
            if b < 0.5 {
                2.0 * b * s
            } else {
                1.0 - 2.0 * (1.0 - b) * (1.0 - s)
            }
        }),
        _ => [0, 1, 2].map(|i| 1.0 - (1.0 - colour[i].min(1.0)) * (1.0 - light[i].min(1.0))),
    };
    let out_alpha = alpha + (1.0 - alpha) * k.min(1.0);
    premultiply(blended, out_alpha)
}

pub(super) fn lens_flare(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let [cx, cy, brightness, blend] = [0, 1, 2, 3].map(|i| uniform(values, i));
    let base = inputs.texel_input(x, y);
    let (w, h) = (inputs.input.width() as f32, inputs.input.height() as f32);
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let scale = w.max(h);
    let d = (px - cx).hypot(py - cy) / scale;
    let core = 0.35 * (-d * 18.0).exp() + 0.08 * (-d * 3.0).exp();
    let mut light = [core; 3];
    let (mx, my) = (w / 2.0, h / 2.0);
    for (index, (t, radius, hue)) in [
        (0.35, 0.05, [0.4, 0.6, 1.0]),
        (0.77, 0.08, [1.0, 0.5, 0.3]),
        (1.19, 0.03, [0.9, 0.4, 1.0]),
    ]
    .into_iter()
    .enumerate()
    {
        let gx = cx + (mx - cx) * 2.0 * t;
        let gy = cy + (my - cy) * 2.0 * t;
        let dd = (px - gx).hypot(py - gy) / scale;
        let ring = ((1.0 - ((dd - radius) / 0.01).abs()).max(0.0) * 0.12
            + if dd < radius { 0.05 } else { 0.0 })
            * if index == 1 { 0.8 } else { 1.0 };
        for k in 0..3 {
            light[k] += hue[k] * ring;
        }
    }
    let gain = brightness * (1.0 - blend);
    let light = light.map(|v| (v * gain).max(0.0));
    let mut out = base;
    for k in 0..3 {
        out[k] = (base[k] + light[k]).max(0.0);
    }
    out[3] = base[3] + (1.0 - base[3]) * light[0].max(light[1]).max(light[2]).min(1.0);
    out
}

pub(super) fn rgb_split(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let mode = uniform(values, 0).round() as u32;
    let [amount, angle, cx, cy, maxd, blend] = [1, 2, 3, 4, 5, 6].map(|i| uniform(values, i));
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let (ox, oy) = if mode == 1 {
        (
            (px - cx) / maxd.max(1.0) * amount,
            (py - cy) / maxd.max(1.0) * amount,
        )
    } else {
        (angle.cos() * amount, angle.sin() * amount)
    };
    let r = inputs.sample_input(px - ox - 0.5, py - oy - 0.5, image_ops::Edge::Clamp);
    let g = texel(inputs.input, x, y);
    let b = inputs.sample_input(px + ox - 0.5, py + oy - 0.5, image_ops::Edge::Clamp);
    let mixed = [r[0], g[1], b[2], r[3].max(g[3]).max(b[3])];
    // `blend` is the original-picture fraction: 0 is the full split, 1 the input.
    std::array::from_fn(|channel| mixed[channel] + (g[channel] - mixed[channel]) * blend)
}

// ------------------------------------------------------------------ overlay and key

pub(super) fn drop_shadow(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let colour = decoded_color(color_at(values, 0));
    let [opacity, dx, dy, only] = [3, 4, 5, 6].map(|i| uniform(values, i));
    let base = inputs.texel_input(x, y);
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let shifted = inputs.sample_current(px - dx - 0.5, py - dy - 0.5, image_ops::Edge::Clamp);
    let alpha = shifted[3] * opacity;
    let shadow = [
        colour[0] * alpha,
        colour[1] * alpha,
        colour[2] * alpha,
        alpha,
    ];
    if only > 0.5 {
        return shadow;
    }
    let k = 1.0 - shadow[3];
    [
        shadow[0] + base[0] * k,
        shadow[1] + base[1] * k,
        shadow[2] + base[2] * k,
        shadow[3] + base[3] * k,
    ]
}

pub(super) fn long_shadow(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let colour = decoded_color(color_at(values, 3));
    let [angle, length, reach, opacity, fade, only, steps] =
        [0, 1, 2, 6, 7, 8, 9].map(|i| uniform(values, i));
    let steps = (steps.round() as u32).clamp(2, 64);
    let base = inputs.texel_input(x, y);
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let (dx, dy) = (angle.sin(), -angle.cos());
    let mut alpha = 0.0;
    for step in 1..=steps {
        let distance = reach * step as f32 / steps as f32;
        let q = inputs.sample_input(
            px - dx * distance - 0.5,
            py - dy * distance - 0.5,
            image_ops::Edge::Transparent,
        );
        if q[3] > 0.5 {
            alpha = (if fade > 0.5 {
                1.0 - distance / length.max(1e-3)
            } else {
                1.0
            })
            .clamp(0.0, 1.0)
                * opacity;
            break;
        }
    }
    let shadow = [
        colour[0] * alpha,
        colour[1] * alpha,
        colour[2] * alpha,
        alpha,
    ];
    if only > 0.5 {
        return shadow;
    }
    let k = 1.0 - shadow[3];
    [
        shadow[0] + base[0] * k,
        shadow[1] + base[1] * k,
        shadow[2] + base[2] * k,
        shadow[3] + base[3] * k,
    ]
}

pub(super) fn gradient(inputs: &SampleInputs<'_>, x: u32, y: u32, values: &[f32]) -> [f32; 4] {
    let _ = inputs;
    let (sx, sy) = (uniform(values, 0), uniform(values, 1));
    let (ex, ey) = (uniform(values, 2), uniform(values, 3));
    let start = decoded_color(color_at(values, 4));
    let end = decoded_color(color_at(values, 7));
    let shape = uniform(values, 10).round() as u32;
    let gamma = uniform(values, 11);
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let (vx, vy) = (ex - sx, ey - sy);
    let l2 = (vx * vx + vy * vy).max(1e-6);
    let l = l2.sqrt();
    let (dx, dy) = (px - sx, py - sy);
    let along = (dx * vx + dy * vy) / l2;
    let across = (-dx * vy + dy * vx) / l2;
    let t = match shape {
        1 => dx.hypot(dy) / l,
        2 => along.abs(),
        3 => along.abs() + across.abs(),
        _ => along,
    };
    let t = t.clamp(0.0, 1.0).powf(gamma);
    let colour = [0, 1, 2].map(|k| start[k] + (end[k] - start[k]) * t);
    premultiply(colour, 1.0)
}

pub(super) fn four_color_gradient(
    inputs: &SampleInputs<'_>,
    x: u32,
    y: u32,
    values: &[f32],
) -> [f32; 4] {
    let points = [
        point_at(values, 0),
        point_at(values, 2),
        point_at(values, 4),
        point_at(values, 6),
    ];
    let colours = [
        color_at(values, 8),
        color_at(values, 11),
        color_at(values, 14),
        color_at(values, 17),
    ];
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let mut acc = [0.0f32; 3];
    let mut weight_sum = 0.0;
    for (point, colour) in points.iter().zip(colours) {
        let d = (px - point[0]).hypot(py - point[1]).max(1.0);
        let weight = 1.0 / d.powi(2);
        for k in 0..3 {
            acc[k] += colour[k] * weight;
        }
        weight_sum += weight;
    }
    let colour = acc.map(|v| v / weight_sum.max(1e-6));
    let _ = inputs;
    premultiply(colour, 1.0)
}

pub(super) fn color_key(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let alpha = pixel[3];
    if alpha <= 0.0 {
        return pixel;
    }
    let key = color_at(values, 0);
    let tolerance = uniform(values, 3);
    let soft = uniform(values, 4).max(1e-4);
    let thin = uniform(values, 5);
    let (colour, _) = straight(pixel);
    let encoded = encoded_color(colour);
    let key_encoded = encoded_color(key);
    let (y, cb, cr) = key_chroma(encoded);
    let (key_y, key_cb, key_cr) = key_chroma(key_encoded);
    let distance =
        ((cb - key_cb).powi(2) + (cr - key_cr).powi(2)).sqrt() + (y - key_y).abs() * 0.15;
    let tolerance = (tolerance + thin / 5.0 * 0.1).max(0.0);
    let matte = ((distance - tolerance) / soft).clamp(0.0, 1.0);
    let out_alpha = alpha * matte;
    [
        colour[0] * out_alpha,
        colour[1] * out_alpha,
        colour[2] * out_alpha,
        out_alpha,
    ]
}

fn key_chroma(colour: [f32; 3]) -> (f32, f32, f32) {
    let y = luma(colour);
    // BT.709 chroma scaling: 2*(1-Kb) and 2*(1-Kr).
    (y, (colour[2] - y) / 1.8556, (colour[0] - y) / 1.5748)
}

pub(super) fn luma_key(pixel: [f32; 4], values: &[f32]) -> [f32; 4] {
    let [threshold, cutoff, invert] = [0, 1, 2].map(|i| uniform(values, i));
    let alpha = pixel[3];
    if alpha <= 0.0 {
        return pixel;
    }
    let (colour, _) = straight(pixel);
    let level = luma(encoded_color(colour)).clamp(0.0, 1.0);
    let level = if invert > 0.5 { 1.0 - level } else { level };
    let matte = if level <= cutoff {
        0.0
    } else if level >= threshold.max(cutoff + 1e-3) {
        1.0
    } else {
        (level - cutoff) / (threshold - cutoff).max(1e-3)
    };
    let out_alpha = alpha * matte.clamp(0.0, 1.0);
    [
        colour[0] * out_alpha,
        colour[1] * out_alpha,
        colour[2] * out_alpha,
        out_alpha,
    ]
}

// ------------------------------------------------------------------ uniform checks

fn warp(values: &[f32], len: usize) -> bool {
    finite(values, len)
}
pub(super) fn valid_twirl(values: &[f32], _: u32, _: u32) -> bool {
    warp(values, 4) && uniform(values, 3) >= 0.0
}
pub(super) fn valid_spherize(values: &[f32], _: u32, _: u32) -> bool {
    warp(values, 3) && uniform(values, 2) >= 0.0
}
pub(super) fn valid_magnify(values: &[f32], _: u32, _: u32) -> bool {
    warp(values, 6)
        && uniform(values, 2) > 0.0
        && uniform(values, 3) >= 0.0
        && uniform(values, 4) >= 0.0
}
pub(super) fn valid_wave(values: &[f32], _: u32, _: u32) -> bool {
    warp(values, 4) && uniform(values, 1) > 0.0
}
pub(super) fn valid_lens(values: &[f32], _: u32, _: u32) -> bool {
    warp(values, 4) && uniform(values, 3) > 0.0
}
pub(super) fn valid_turbulent(values: &[f32], _: u32, _: u32) -> bool {
    warp(values, 8) && uniform(values, 2) > 0.0 && (1.0..=10.0).contains(&uniform(values, 5))
}
pub(super) fn valid_corner(values: &[f32], _: u32, _: u32) -> bool {
    warp(values, 11) && uniform(values, 9) > 0.0 && uniform(values, 10) > 0.0
}
pub(super) fn valid_basic_3d(values: &[f32], _: u32, _: u32) -> bool {
    warp(values, 6)
        && uniform(values, 3) > 0.0
        && uniform(values, 4) > 0.0
        && uniform(values, 5) > 0.0
}
pub(super) fn valid_brush(values: &[f32], _: u32, _: u32) -> bool {
    warp(values, 5) && uniform(values, 3) > 0.0 && (2.0..=24.0).contains(&uniform(values, 4))
}
pub(super) fn valid_roughen(values: &[f32], _: u32, _: u32) -> bool {
    warp(values, 8) && uniform(values, 5) > 0.0 && (1.0..=10.0).contains(&uniform(values, 6))
}
pub(super) fn valid_shake(values: &[f32], _: u32, _: u32) -> bool {
    warp(values, 5) && uniform(values, 3) > 0.0
}
pub(super) fn valid_sobel(values: &[f32], _: u32, _: u32) -> bool {
    finite(values, 1) && (0.0..=1.0).contains(&uniform(values, 0))
}
pub(super) fn valid_glow(values: &[f32], _: u32, _: u32) -> bool {
    finite(values, 8) && uniform(values, 4) >= 0.0
}
pub(super) fn valid_glint(values: &[f32], _: u32, _: u32) -> bool {
    warp(values, 9)
        && (2.0..=8.0).contains(&uniform(values, 6))
        && (2.0..=64.0).contains(&uniform(values, 8))
}
pub(super) fn valid_radial(values: &[f32], _: u32, _: u32) -> bool {
    warp(values, 9) && (4.0..=64.0).contains(&uniform(values, 8))
}
pub(super) fn valid_leaks(values: &[f32], _: u32, _: u32) -> bool {
    warp(values, 12) && uniform(values, 7) > 0.0
}
pub(super) fn valid_flare(values: &[f32], _: u32, _: u32) -> bool {
    warp(values, 4) && uniform(values, 2) >= 0.0
}
pub(super) fn valid_split(values: &[f32], _: u32, _: u32) -> bool {
    warp(values, 7) && uniform(values, 5) > 0.0
}
pub(super) fn valid_drop(values: &[f32], _: u32, _: u32) -> bool {
    warp(values, 7)
}
pub(super) fn valid_long(values: &[f32], _: u32, _: u32) -> bool {
    warp(values, 10) && (2.0..=64.0).contains(&uniform(values, 9))
}
pub(super) fn valid_gradient(values: &[f32], _: u32, _: u32) -> bool {
    warp(values, 12) && uniform(values, 11) > 0.0
}
pub(super) fn valid_four(values: &[f32], _: u32, _: u32) -> bool {
    warp(values, 20)
}
pub(super) fn valid_color_key(values: &[f32]) -> bool {
    finite(values, 6) && uniform(values, 4) > 0.0
}
pub(super) fn valid_luma_key(values: &[f32]) -> bool {
    finite(values, 3)
}

// ------------------------------------------------------------------ evaluators

fn encoded(colour: [f64; 4]) -> [f32; 3] {
    [colour[0] as f32, colour[1] as f32, colour[2] as f32]
}
/// A straight sRGB colour parameter in linear light, for light buffers.
fn linear(colour: [f64; 4]) -> [f32; 3] {
    decoded_color(encoded(colour))
}
fn floats(values: impl IntoIterator<Item = f32>) -> Vec<Uniform> {
    values.into_iter().map(Uniform::Float).collect()
}
fn working_geometry(frame: &EffectFrame, geometry: &EffectGeometry) -> Result<[f32; 2]> {
    if geometry.working_size() != [frame.width, frame.height] {
        return Err(Error::Invalid("effect spatial geometry"));
    }
    Ok([frame.width as f32, frame.height as f32])
}

pub(super) fn twirl_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    let [w, h] = working_geometry(frame, geometry)?;
    let angle = frame.scalar("angle")?.to_radians() as f32;
    let radius = frame.scalar("radius")? as f32 / 100.0 * w.min(h);
    let point = frame.point("center")?;
    let cx = point[0] as f32 * w;
    let cy = point[1] as f32 * h;
    if angle == 0.0 || radius < 0.5 {
        return Ok(Vec::new());
    }
    Ok(vec![EffectPass::Sample {
        entry_point: "twirl",
        uniforms: floats([cx, cy, angle, radius]),
    }])
}

pub(super) fn spherize_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    let [w, h] = working_geometry(frame, geometry)?;
    let [gx, gy] = geometry.clip_pixel_scale();
    let radius = (frame.scalar("radius")? * gx.min(gy)) as f32;
    let point = frame.point("center")?;
    let [cx, cy] = [point[0] as f32 * w, point[1] as f32 * h];
    if radius < 0.5 {
        return Ok(Vec::new());
    }
    Ok(vec![EffectPass::Sample {
        entry_point: "spherize",
        uniforms: floats([cx, cy, radius]),
    }])
}

pub(super) fn magnify_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    let [w, h] = working_geometry(frame, geometry)?;
    let [gx, gy] = geometry.clip_pixel_scale();
    let point = frame.point("center")?;
    let [cx, cy] = [point[0] as f32 * w, point[1] as f32 * h];
    let magnification = frame.scalar("magnification")? as f32;
    let size = (frame.scalar("size")? * f64::from(gx.min(gy))) as f32;
    let feather = (frame.scalar("feather")? * f64::from(gx.min(gy))) as f32;
    Ok(vec![EffectPass::Sample {
        entry_point: "magnify",
        uniforms: floats([
            cx,
            cy,
            magnification,
            size,
            feather,
            frame.choice("shape")? as f32,
        ]),
    }])
}

pub(super) fn wave_eval(frame: &EffectFrame, geometry: &EffectGeometry) -> Result<Vec<EffectPass>> {
    let [_, _] = working_geometry(frame, geometry)?;
    let [gx, gy] = geometry.clip_pixel_scale();
    let gain = (gx + gy) / 2.0;
    let height = (frame.scalar("height")? * gain) as f32;
    let width = (frame.scalar("width")? * gain) as f32;
    let direction = frame.scalar("direction")?.to_radians() as f32;
    let phase = (frame.seconds * frame.scalar("speed")? * std::f64::consts::TAU) as f32;
    if height == 0.0 {
        return Ok(Vec::new());
    }
    Ok(vec![EffectPass::Sample {
        entry_point: "wave_warp",
        uniforms: floats([height, width, direction, phase]),
    }])
}

pub(super) fn lens_eval(frame: &EffectFrame, geometry: &EffectGeometry) -> Result<Vec<EffectPass>> {
    let [w, h] = working_geometry(frame, geometry)?;
    let curvature = frame.scalar("curvature")? as f32 / 100.0;
    let v = frame.scalar("v_decentering")? as f32 / 100.0;
    let hd = frame.scalar("h_decentering")? as f32 / 100.0;
    if curvature == 0.0 && v == 0.0 && hd == 0.0 {
        return Ok(Vec::new());
    }
    let cx = w / 2.0 + hd * w / 2.0;
    let cy = h / 2.0 + v * h / 2.0;
    let norm = (w / 2.0).hypot(h / 2.0).max(1e-6);
    Ok(vec![EffectPass::Sample {
        entry_point: "lens_distortion",
        uniforms: floats([curvature, cx, cy, norm]),
    }])
}

pub(super) fn turbulent_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    let [w, h] = working_geometry(frame, geometry)?;
    let [gx, gy] = geometry.clip_pixel_scale();
    let gain = (gx + gy) / 2.0;
    let amount = (frame.scalar("amount")? * gain) as f32;
    let size = (frame.scalar("size")? * gain).max(0.5) as f32;
    let point = frame.point("offset")?;
    let offset = [
        ((point[0] as f32 * w - w / 2.0) / size),
        ((point[1] as f32 * h - h / 2.0) / size),
    ];
    let complexity = frame.scalar("complexity")? as f32;
    let evolution = frame.scalar("evolution")?.to_radians() as f32 / TAU;
    if amount == 0.0 {
        return Ok(Vec::new());
    }
    Ok(vec![EffectPass::Sample {
        entry_point: "turbulent_displace",
        uniforms: floats([
            frame.choice("displacement")? as f32,
            amount,
            size,
            offset[0],
            offset[1],
            complexity,
            evolution,
            if frame.fine { 0.0 } else { 1.0 },
        ]),
    }])
}

fn square_to_quad(q: [[f64; 2]; 4]) -> [f64; 9] {
    let [p0, p1, p2, p3] = q;
    let sx = p0[0] - p1[0] + p2[0] - p3[0];
    let sy = p0[1] - p1[1] + p2[1] - p3[1];
    if sx.abs() < 1e-12 && sy.abs() < 1e-12 {
        return [
            p1[0] - p0[0],
            p2[0] - p1[0],
            p0[0],
            p1[1] - p0[1],
            p2[1] - p1[1],
            p0[1],
            0.0,
            0.0,
            1.0,
        ];
    }
    let (dx1, dx2, dy1, dy2) = (p1[0] - p2[0], p3[0] - p2[0], p1[1] - p2[1], p3[1] - p2[1]);
    let det = dx1 * dy2 - dx2 * dy1;
    if det.abs() < 1e-18 {
        return [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    }
    let g = (sx * dy2 - dx2 * sy) / det;
    let h = (dx1 * sy - sx * dy1) / det;
    [
        p1[0] - p0[0] + g * p1[0],
        p3[0] - p0[0] + h * p3[0],
        p0[0],
        p1[1] - p0[1] + g * p1[1],
        p3[1] - p0[1] + h * p3[1],
        p0[1],
        g,
        h,
        1.0,
    ]
}
fn h_inverse(m: &[f64; 9]) -> Option<[f64; 9]> {
    let [a, b, c, d, e, f, g, h, i] = *m;
    let co = [
        e * i - f * h,
        -(d * i - f * g),
        d * h - e * g,
        -(b * i - c * h),
        a * i - c * g,
        -(a * h - b * g),
        b * f - c * e,
        -(a * f - c * d),
        a * e - b * d,
    ];
    let det = a * co[0] + b * co[1] + c * co[2];
    if det.abs() < 1e-18 {
        return None;
    }
    Some([
        co[0] / det,
        co[3] / det,
        co[6] / det,
        co[1] / det,
        co[4] / det,
        co[7] / det,
        co[2] / det,
        co[5] / det,
        co[8] / det,
    ])
}

pub(super) fn corner_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    let [w, h] = working_geometry(frame, geometry)?;
    let names = ["upper_left", "upper_right", "lower_right", "lower_left"];
    let mut points = [[0.0f64; 2]; 4];
    for (slot, name) in points.iter_mut().zip(names) {
        let point = frame.point(name)?;
        *slot = [point[0] * f64::from(w), point[1] * f64::from(h)];
    }
    let identity = [
        [0.0, 0.0],
        [f64::from(w), 0.0],
        [f64::from(w), f64::from(h)],
        [0.0, f64::from(h)],
    ];
    if points
        .iter()
        .zip(identity)
        .all(|(a, b)| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6)
    {
        return Ok(Vec::new());
    }
    let Some(inverse) = h_inverse(&square_to_quad(points)) else {
        return Ok(Vec::new());
    };
    let mut values: Vec<f32> = inverse.map(|v| v as f32).to_vec();
    values.push(w);
    values.push(h);
    Ok(vec![EffectPass::Sample {
        entry_point: "corner_pin",
        uniforms: floats(values),
    }])
}

pub(super) fn basic_3d_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    let [w, h] = working_geometry(frame, geometry)?;
    let [gx, gy] = geometry.clip_pixel_scale();
    let swivel = frame.scalar("swivel")?.to_radians() as f32;
    let tilt = frame.scalar("tilt")?.to_radians() as f32;
    let distance = (frame.scalar("distance")? * gx.min(gy)) as f32;
    if swivel == 0.0 && tilt == 0.0 && distance == 0.0 {
        return Ok(Vec::new());
    }
    let focal = w.max(h) * 1.2;
    Ok(vec![EffectPass::Sample {
        entry_point: "basic_3d",
        uniforms: floats([swivel, tilt, distance, focal, w, h]),
    }])
}

pub(super) fn brush_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    let [_, _] = working_geometry(frame, geometry)?;
    let [gx, gy] = geometry.clip_pixel_scale();
    let gain = (gx + gy) / 2.0;
    let angle = frame.scalar("angle")?.to_radians() as f32;
    let size = (frame.scalar("size")? * gain) as f32;
    let length = (frame.scalar("length")? * gain) as f32;
    let density = frame.scalar("density")?.max(0.05) as f32;
    if length < 0.5 && size < 0.5 {
        return Ok(Vec::new());
    }
    let cell = (length.max(2.0) / density).max(2.0);
    let steps = (length.ceil() as u32).clamp(2, 24);
    let steps = if frame.fine { steps } else { steps.min(8) };
    Ok(vec![EffectPass::Sample {
        entry_point: "brush_strokes",
        uniforms: floats([angle, size, length, cell, steps as f32]),
    }])
}

pub(super) fn roughen_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    let [_, _] = working_geometry(frame, geometry)?;
    let [gx, gy] = geometry.clip_pixel_scale();
    let border = (frame.scalar("border")? * gx.min(gy)) as f32;
    if border < 0.5 {
        return Ok(Vec::new());
    }
    let scale = (frame.scalar("scale")? / 100.0 * 30.0 * f64::from(gx.min(gy))).max(0.5) as f32;
    let mut passes = vec![EffectPass::Pixel {
        entry_point: "alpha_plane",
        uniforms: Vec::new(),
    }];
    passes.extend(super::sigma_passes(
        (border * 0.5).clamp(0.0, 64.0),
        frame.width,
        frame.height,
    )?);
    passes.push(EffectPass::Sample {
        entry_point: "roughen_edges",
        uniforms: floats([
            frame.choice("edge_type")? as f32,
            0.8,
            0.5,
            0.2,
            border,
            scale,
            frame.scalar("complexity")? as f32,
            if frame.fine { 0.0 } else { 1.0 },
        ]),
    });
    Ok(passes)
}

pub(super) fn shake_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    let [_, _] = working_geometry(frame, geometry)?;
    let [gx, gy] = geometry.clip_pixel_scale();
    let amount = (frame.scalar("amount")? * gx.min(gy)) as f32;
    let rotation = frame.scalar("rotation")?.to_radians() as f32;
    let zoom = frame.scalar("zoom")? as f32 / 100.0;
    let frequency = frame.scalar("frequency")? as f32;
    Ok(vec![EffectPass::Sample {
        entry_point: "camera_shake",
        uniforms: floats([amount, rotation, zoom, frequency, frame.seconds as f32]),
    }])
}

pub(super) fn posterize_time_passes(
    _: &EffectFrame,
    _: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    Ok(Vec::new())
}

pub(super) fn posterize_time_content(
    frame: &EffectFrame,
    content: time::Tick,
) -> Result<time::Tick> {
    let rate = frame.scalar("rate")?.max(0.01);
    let held = (content.seconds() * rate).floor() / rate;
    Ok(time::Tick::from_seconds_f64(held, Rounding::Floor)?)
}

fn highlight_passes(threshold: f64, soft: f32) -> Vec<EffectPass> {
    vec![EffectPass::Pixel {
        entry_point: "highlights",
        uniforms: floats([(threshold / 100.0) as f32, soft]),
    }]
}
fn glow_add_pass(
    colour: [f32; 3],
    intensity: f32,
    saturation: f32,
    tint: f32,
    screen: f32,
    only: f32,
) -> EffectPass {
    EffectPass::Sample {
        entry_point: "glow_add",
        uniforms: floats([
            colour[0], colour[1], colour[2], intensity, saturation, tint, screen, only,
        ]),
    }
}

pub(super) fn echo_glow_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    working_geometry(frame, geometry)?;
    // Radius is in clip pixels like every other glow: scale it to the output so
    // a preview tile and an export blur the same fraction of the picture.
    let [gx, gy] = geometry.clip_pixel_scale();
    let gain = (gx + gy) / 2.0;
    let colour = linear(frame.color("color")?);
    let echoes = frame.scalar("echoes")? as f32;
    let radius = frame.scalar("radius")? * gain;
    let spread = frame.scalar("spread")? as f32;
    let intensity = frame.scalar("intensity")? as f32 / 100.0;
    let decay = frame.scalar("decay")? as f32 / 100.0;
    let count = if frame.fine {
        echoes.min(6.0)
    } else {
        echoes.min(3.0)
    };
    let mut passes = highlight_passes(frame.scalar("threshold")?, 0.1);
    let base = radius * 2.0;
    for echo in 0..count.max(1.0).round() as u32 {
        let sigma = (base as f32 * spread.powi(echo as i32) / echo.max(1) as f32).clamp(0.0, 64.0);
        passes.extend(super::sigma_passes(sigma, frame.width, frame.height)?);
    }
    let weight = (1.0 - decay).powi(count.max(1.0) as i32);
    passes.push(glow_add_pass(
        colour,
        intensity * 2.0 * weight.max(0.05),
        1.0,
        1.0,
        0.0,
        0.0,
    ));
    Ok(passes)
}

pub(super) fn edge_glow_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    working_geometry(frame, geometry)?;
    let [gx, gy] = geometry.clip_pixel_scale();
    let gain = ((gx + gy) / 2.0) as f32;
    let colour = linear(frame.color("color")?);
    let width = (frame.scalar("width")? as f32 * gain).max(0.0);
    let radius = (frame.scalar("radius")? as f32 * gain * 0.5).max(0.0);
    let intensity = frame.scalar("intensity")? as f32 / 100.0;
    let only = if frame.flag("only")? { 1.0 } else { 0.0 };
    let mut passes = vec![EffectPass::Sample {
        entry_point: "sobel_edges",
        uniforms: floats([(frame.scalar("threshold")? / 100.0) as f32]),
    }];
    if frame.fine && width > 0.5 {
        passes.extend(super::sigma_passes(
            (width * 0.5).min(32.0),
            frame.width,
            frame.height,
        )?);
    }
    passes.extend(super::sigma_passes(
        radius.min(64.0),
        frame.width,
        frame.height,
    )?);
    passes.push(glow_add_pass(colour, intensity * 2.2, 1.0, 1.0, 0.0, only));
    Ok(passes)
}

pub(super) fn glint_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    working_geometry(frame, geometry)?;
    let [gx, gy] = geometry.clip_pixel_scale();
    let gain = ((gx + gy) / 2.0) as f32;
    let colour = linear(frame.color("color")?);
    let length = (frame.scalar("length")? as f32 * gain).max(0.0);
    let rays = [2.0, 4.0, 6.0, 8.0][(frame.choice("rays")? as usize).min(3)];
    let steps = if frame.fine { 24.0 } else { 10.0 };
    let mut passes = highlight_passes(frame.scalar("threshold")?, 0.08);
    passes.push(EffectPass::Sample {
        entry_point: "glint_rays",
        uniforms: floats([
            colour[0],
            colour[1],
            colour[2],
            frame.scalar("intensity")? as f32 / 100.0 * 3.0,
            length,
            frame.scalar("rotation")?.to_radians() as f32,
            rays,
            frame.scalar("colorize")? as f32 / 100.0,
            steps,
        ]),
    });
    Ok(passes)
}

pub(super) fn wonder_glow_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    working_geometry(frame, geometry)?;
    let [gx, gy] = geometry.clip_pixel_scale();
    let gain = ((gx + gy) / 2.0) as f32;
    let colour = linear(frame.color("color")?);
    let radius = (frame.scalar("radius")? as f32 * gain * 0.5).max(0.0);
    let intensity = frame.scalar("intensity")? as f32 / 100.0;
    let saturation = frame.scalar("saturation")? as f32 / 100.0;
    let tint = if frame.flag("use_color")? { 1.0 } else { 0.0 };
    let screen = if frame.choice("mode")? == 1 { 1.0 } else { 0.0 };
    let mut passes = highlight_passes(frame.scalar("threshold")?, 0.15);
    let scales: &[f32] = if frame.fine {
        &[0.25, 1.0, 3.0]
    } else {
        &[1.0]
    };
    for scale in scales {
        passes.extend(super::sigma_passes(
            (radius * scale).min(64.0),
            frame.width,
            frame.height,
        )?);
    }
    passes.push(glow_add_pass(
        colour,
        intensity * 1.5,
        saturation,
        tint,
        screen,
        0.0,
    ));
    Ok(passes)
}

pub(super) fn radial_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    let [w, h] = working_geometry(frame, geometry)?;
    let colour = linear(frame.color("color")?);
    let point = frame.point("center")?;
    let [cx, cy] = [point[0] as f32 * w, point[1] as f32 * h];
    let steps = if frame.fine { 48.0 } else { 12.0 };
    let only = if frame.flag("only")? { 1.0 } else { 0.0 };
    let mut passes = highlight_passes(frame.scalar("threshold")?, 0.1);
    passes.push(EffectPass::Sample {
        entry_point: "radial_rays",
        uniforms: floats([
            cx,
            cy,
            frame.scalar("length")? as f32 / 100.0,
            frame.scalar("intensity")? as f32 / 100.0,
            colour[0],
            colour[1],
            colour[2],
            only,
            steps,
        ]),
    });
    Ok(passes)
}

pub(super) fn leaks_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    working_geometry(frame, geometry)?;
    let [gx, gy] = geometry.clip_pixel_scale();
    let gain = ((gx + gy) / 2.0) as f32;
    let c1 = linear(frame.color("c1")?);
    let c2 = linear(frame.color("c2")?);
    let scale = (frame.scalar("scale")? as f32 / 100.0).max(0.05)
        * frame.width.max(frame.height) as f32
        * 0.6;
    let phase = (frame.seconds * frame.scalar("speed")?) as f32;
    Ok(vec![EffectPass::Sample {
        entry_point: "light_leaks",
        uniforms: floats([
            c1[0],
            c1[1],
            c1[2],
            c2[0],
            c2[1],
            c2[2],
            frame.scalar("intensity")? as f32 / 100.0,
            scale * gain,
            frame.scalar("direction")?.to_radians() as f32,
            phase,
            frame.scalar("seed")? as f32,
            frame.choice("mode")? as f32,
        ]),
    }])
}

pub(super) fn flare_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    let [w, h] = working_geometry(frame, geometry)?;
    let point = frame.point("center")?;
    let [cx, cy] = [point[0] as f32 * w, point[1] as f32 * h];
    let brightness = frame.scalar("brightness")? as f32 / 100.0;
    let blend = frame.scalar("blend")? as f32 / 100.0;
    if brightness == 0.0 || blend >= 1.0 {
        return Ok(Vec::new());
    }
    Ok(vec![EffectPass::Sample {
        entry_point: "lens_flare",
        uniforms: floats([cx, cy, brightness, blend]),
    }])
}

pub(super) fn split_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    let [w, h] = working_geometry(frame, geometry)?;
    let [gx, gy] = geometry.clip_pixel_scale();
    let amount = (frame.scalar("amount")? * gx.min(gy)) as f32;
    if amount < 1e-3 {
        return Ok(Vec::new());
    }
    let point = frame.point("center")?;
    let [cx, cy] = [point[0] as f32 * w, point[1] as f32 * h];
    Ok(vec![EffectPass::Sample {
        entry_point: "rgb_split",
        uniforms: floats([
            frame.choice("mode")? as f32,
            amount,
            frame.scalar("angle")?.to_radians() as f32,
            cx,
            cy,
            (w.hypot(h) / 2.0).max(1.0),
            frame.scalar("blend")? as f32 / 100.0,
        ]),
    }])
}

pub(super) fn drop_eval(frame: &EffectFrame, geometry: &EffectGeometry) -> Result<Vec<EffectPass>> {
    working_geometry(frame, geometry)?;
    let [gx, gy] = geometry.clip_pixel_scale();
    let gain = (gx + gy) / 2.0;
    let colour = encoded(frame.color("color")?);
    let angle = frame.scalar("direction")?.to_radians();
    let distance = frame.scalar("distance")? * gain;
    let softness = frame.scalar("softness")? * gain * 0.5;
    let mut passes = Vec::new();
    if softness > 0.3 {
        passes.extend(super::sigma_passes(
            (softness as f32).min(64.0),
            frame.width,
            frame.height,
        )?);
    }
    passes.push(EffectPass::Sample {
        entry_point: "drop_shadow",
        uniforms: floats([
            colour[0],
            colour[1],
            colour[2],
            frame.scalar("opacity")? as f32 / 100.0,
            (angle.sin() * distance) as f32,
            (-angle.cos() * distance) as f32,
            if frame.flag("only")? { 1.0 } else { 0.0 },
        ]),
    });
    Ok(passes)
}

pub(super) fn long_eval(frame: &EffectFrame, geometry: &EffectGeometry) -> Result<Vec<EffectPass>> {
    let [w, h] = working_geometry(frame, geometry)?;
    let [gx, gy] = geometry.clip_pixel_scale();
    let gain = (gx + gy) / 2.0;
    let colour = encoded(frame.color("color")?);
    let angle = frame.scalar("angle")?.to_radians();
    let length = (frame.scalar("length")? * gain) as f32;
    let reach = length.min(w + h);
    if reach < 0.5 {
        return Ok(Vec::new());
    }
    Ok(vec![EffectPass::Sample {
        entry_point: "long_shadow",
        uniforms: floats([
            angle as f32,
            length,
            reach,
            colour[0],
            colour[1],
            colour[2],
            frame.scalar("opacity")? as f32 / 100.0,
            if frame.flag("fade")? { 1.0 } else { 0.0 },
            if frame.flag("only")? { 1.0 } else { 0.0 },
            if frame.fine { 40.0 } else { 12.0 },
        ]),
    }])
}

pub(super) fn gradient_eval(
    frame: &EffectFrame,
    geometry: &EffectGeometry,
) -> Result<Vec<EffectPass>> {
    let [w, h] = working_geometry(frame, geometry)?;
    let start = frame.point("start")?;
    let end = frame.point("end")?;
    let start = [start[0] as f32 * w, start[1] as f32 * h];
    let end = [end[0] as f32 * w, end[1] as f32 * h];
    let sc = encoded(frame.color("start_color")?);
    let ec = encoded(frame.color("end_color")?);
    let gamma = 1.0;
    Ok(vec![EffectPass::Sample {
        entry_point: "gradient",
        uniforms: floats([
            start[0],
            start[1],
            end[0],
            end[1],
            sc[0],
            sc[1],
            sc[2],
            ec[0],
            ec[1],
            ec[2],
            frame.choice("shape")? as f32,
            gamma,
        ]),
    }])
}

pub(super) fn four_eval(frame: &EffectFrame, geometry: &EffectGeometry) -> Result<Vec<EffectPass>> {
    let [w, h] = working_geometry(frame, geometry)?;
    let mut values = Vec::with_capacity(20);
    for name in ["p1", "p2", "p3", "p4"] {
        let point = frame.point(name)?;
        values.push(point[0] as f32 * w);
        values.push(point[1] as f32 * h);
    }
    for name in ["c1", "c2", "c3", "c4"] {
        values.extend(encoded(frame.color(name)?));
    }
    Ok(vec![EffectPass::Sample {
        entry_point: "four_color_gradient",
        uniforms: floats(values),
    }])
}

pub(super) fn color_key_eval(frame: &EffectFrame, _: &EffectGeometry) -> Result<Vec<EffectPass>> {
    let colour = encoded(frame.color("color")?);
    let tolerance = frame.scalar("tolerance")? as f32 / 255.0 * 0.6 + 0.01;
    let feather = frame.scalar("feather")? as f32 / 50.0 * 0.2 + 0.01;
    let thin = frame.scalar("thin")? as f32;
    Ok(vec![EffectPass::Pixel {
        entry_point: "color_key",
        uniforms: floats([colour[0], colour[1], colour[2], tolerance, feather, thin]),
    }])
}

pub(super) fn luma_key_eval(frame: &EffectFrame, _: &EffectGeometry) -> Result<Vec<EffectPass>> {
    Ok(vec![EffectPass::Pixel {
        entry_point: "luma_key",
        uniforms: floats([
            frame.scalar("threshold")? as f32 / 100.0,
            frame.scalar("cutoff")? as f32 / 100.0,
            if frame.flag("invert")? { 1.0 } else { 0.0 },
        ]),
    }])
}
