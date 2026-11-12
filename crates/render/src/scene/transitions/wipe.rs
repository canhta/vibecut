//! Shape wipes driven by a scalar field: the incoming clip appears where `field < T(p)`.
use super::pixels::{
    FRAC_PI_2, PI, TAU, angle_deg, angle_dir, centre, ease_out, hash, mix, px, smoothstep,
};
use super::{Out, Tx};
use crate::{Error, ExecutionHooks, Result, check};

/// Edge styling read from the shared wipe parameters.
#[derive(Clone, Copy, Debug)]
pub(super) struct Style {
    feather: f32,
    border: f32,
    border_color: [f32; 4],
    aa: f32,
}
impl Style {
    /// Working-pixel feather/border widths and AA extent, with premultiplied
    /// linear border colour. Shape consumers use their scaled softness, zero
    /// border and AA=1; no catalogue parameters are synthesized here.
    pub(super) fn new(feather: f32, border: f32, border_color: [f32; 4], aa: f32) -> Result<Self> {
        let alpha = border_color[3];
        if [feather, border, aa]
            .iter()
            .any(|v| !v.is_finite() || *v < 0.)
            || !(feather + border + 2. * aa + 1.).is_finite()
            || border_color
                .iter()
                .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
            || border_color[..3].iter().any(|v| *v > alpha)
        {
            return Err(Error::Invalid("transition wipe style"));
        }
        Ok(Self {
            feather,
            border,
            border_color,
            aa,
        })
    }
    fn of(t: &Tx) -> Result<Self> {
        let aa = [0.5, 1., 1.75, 2.5][t.params.choice("antialias")?.min(3) as usize]
            * t.scale.clamp(0.25, 1.);
        Self::new(
            px(t, "feather")?,
            px(t, "border_width")?,
            t.params.color("border_color")?,
            aa,
        )
    }
    fn margin(self) -> f32 {
        self.border + self.feather + 2. * self.aa + 1.
    }
}

/// Coverage of (incoming, incoming+border) at signed distance `d`; negative is incoming.
fn coverage(style: &Style, d: f32) -> (f32, f32) {
    let s = (style.feather + style.aa) * 0.5;
    let kb = 1. - smoothstep(-s, s, d);
    let ko = if style.border > 0. {
        1. - smoothstep(style.border - s, style.border + s, d)
    } else {
        kb
    };
    (kb, ko.max(kb))
}

/// Composite at signed distance in working pixels (negative reveals incoming).
/// A sibling's custom sweep must call this through Out::paint for cancellation.
pub(super) fn wipe_px(style: &Style, a: [f32; 4], b: [f32; 4], d: f32) -> [f32; 4] {
    let (kb, ko) = coverage(style, d);
    std::array::from_fn(|k| {
        a.get(k).copied().unwrap_or(0.) * (1. - ko)
            + style.border_color.get(k).copied().unwrap_or(0.) * (ko - kb)
            + b.get(k).copied().unwrap_or(0.) * kb
    })
}

/// Min/max of `field` over a coarse grid covering the frame.
fn field_range(
    w: f32,
    h: f32,
    field: &(impl Fn(f32, f32) -> f32 + Sync),
    hooks: &dyn ExecutionHooks,
) -> Result<(f32, f32)> {
    let n = 48;
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for j in 0..=n {
        check(hooks)?;
        for i in 0..=n {
            let v = field(w * i as f32 / n as f32, h * j as f32 / n as f32);
            if v.is_finite() {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
    }
    Ok(if lo.is_finite() { (lo, hi) } else { (0., 1.) })
}

/// The sweeping wipe: incoming where `field < T(p)`, field in pixels.
pub(super) fn field_wipe(
    t: &Tx,
    style: Style,
    field: impl Fn(f32, f32) -> f32 + Sync,
    out: &mut Out,
) -> Result<()> {
    let (w, h) = (t.width as f32, t.height as f32);
    let (lo, hi) = field_range(w, h, &field, out.hooks())?;
    let margin = style.margin() + (hi - lo) * 0.01;
    let thr = lo - margin + t.progress * (hi - lo + 2. * margin);
    out.paint(move |x, y| {
        wipe_px(
            &style,
            t.outgoing_at(x, y),
            t.incoming_at(x, y),
            field(x as f32 + 0.5, y as f32 + 0.5) - thr,
        )
    })
}

/// Shape metric in the caller's coordinate units: circle, square, diamond,
/// star, heart, triangle, hexagon (catalogue choices 0..=6). A value below R
/// lies inside the shape of radius R. Callers validate the choice and coordinates.
pub(super) fn shape_dist(shape: u32, x: f32, y: f32) -> f32 {
    let r = (x * x + y * y).sqrt();
    let theta = (-y).atan2(x);
    let polygon = |n: f32, rotation: f32| {
        let segment = TAU / n;
        let a = (theta - rotation).rem_euclid(segment) - segment / 2.;
        r * a.cos() / (PI / n).cos()
    };
    match shape {
        1 => x.abs().max(y.abs()),
        2 => (x.abs() + y.abs()) * std::f32::consts::FRAC_1_SQRT_2,
        3 => star_dist(x, y, 5, 0.45),
        4 => r / (0.5 * (1. - theta.sin()) + 0.08) * 0.5,
        5 => polygon(3., FRAC_PI_2 - PI / 3.),
        6 => polygon(6., 0.),
        _ => r,
    }
}

fn star_dist(x: f32, y: f32, n: u32, inner: f32) -> f32 {
    let r = (x * x + y * y).sqrt();
    if r < 1e-6 {
        return 0.;
    }
    let th = (-y).atan2(x) - FRAC_PI_2;
    let seg = TAU / n.max(2) as f32;
    let a = (th.rem_euclid(seg) - seg / 2.).abs();
    let a = seg / 2. - a;
    let ri = inner.clamp(0.05, 1.);
    let (p1x, p1y) = (ri * (seg / 2.).cos(), ri * (seg / 2.).sin());
    let (ex, ey) = (p1x - 1., p1y);
    let (dx, dy) = (a.cos(), a.sin());
    let denom = dx * ey - dy * ex;
    let tr = if denom.abs() < 1e-9 { 1. } else { ey / denom };
    r / tr.max(0.05)
}

fn rot(x: f32, y: f32, degrees: f32) -> (f32, f32) {
    let (s, c) = degrees.to_radians().sin_cos();
    (x * c + y * s, -x * s + y * c)
}

pub(super) fn run(id: &str, t: &Tx, out: &mut Out) -> Result<()> {
    if id == "neon_edge_wipe" {
        return neon_edge(t, out);
    }
    if id == "stretch_wipe" {
        return stretch(t, out);
    }
    if id == "shape_dissolve" {
        return shape_dissolve(t, out);
    }
    if id == "shape_flow" {
        return shape_flow(t, out);
    }
    let style = Style::of(t)?;
    let (w, h) = (t.width as f32, t.height as f32);
    let aspect = w / h.max(1.);
    match id {
        "straight_wipe" | "soft_edge_wipe" => {
            let (dx, dy) = angle_dir(angle_deg(t, "angle")?);
            field_wipe(t, style, move |x, y| x * dx + y * dy, out)
        }
        "clock_wipe" => {
            let (cx, cy) = centre(t)?;
            let start = t.params.angle("start")?;
            let scale = (w + h) * 0.5;
            field_wipe(
                t,
                style,
                move |x, y| {
                    let a = (x - cx).atan2(-(y - cy)) - start;
                    a.rem_euclid(TAU) / TAU * scale
                },
                out,
            )
        }
        "corner_sweep" => {
            let corner = t.params.choice("corner")?;
            let (ox, oy, a0) = match corner {
                1 => (w, 0., FRAC_PI_2),
                2 => (w, h, PI),
                3 => (0., h, -FRAC_PI_2),
                _ => (0., 0., 0.),
            };
            let scale = (w + h) * 0.5;
            field_wipe(
                t,
                style,
                move |x, y| {
                    let a = (y - oy).atan2(x - ox) - a0;
                    let a = a.rem_euclid(TAU);
                    let a = if a > PI * 1.5 { 0. } else { a.min(FRAC_PI_2) };
                    a / FRAC_PI_2 * scale
                },
                out,
            )
        }
        "star_wipe" => {
            let (cx, cy) = centre(t)?;
            let n = t.params.num("points")?.round().clamp(3., 24.) as u32;
            let inner = t.params.fraction("inner")?;
            let rotation = angle_deg(t, "rotation")?;
            field_wipe(
                t,
                style,
                move |x, y| {
                    let (u, v) = rot(x - cx, y - cy, rotation);
                    star_dist(u, v, n, inner)
                },
                out,
            )
        }
        "panel_wipe" => {
            let choice = t.params.choice("direction")?;
            let (dx, dy): (f32, f32) = match choice {
                0 => (0., 1.),
                1 => (-1., 0.),
                2 => (0., -1.),
                _ => (1., 0.),
            };
            let n = t.params.num("panels")?.round().clamp(1., 64.);
            let along_len = (dx.abs() * w + dy.abs() * h).max(1.);
            let across_len = if dx != 0. { h } else { w };
            field_wipe(
                t,
                style,
                move |x, y| {
                    let along = x * dx + y * dy;
                    let across = if dx != 0. { y } else { x };
                    let idx = (across / across_len * n).floor().clamp(0., n - 1.);
                    along + idx * along_len * 0.6 / n
                },
                out,
            )
        }
        "stair_wipe" => {
            let (dx, dy) = angle_dir(angle_deg(t, "angle")?);
            let n = t.params.num("plateaus")?.round().clamp(1., 64.);
            let (lo, hi) = field_range(w, h, &|x: f32, y: f32| -x * dy + y * dx, out.hooks())?;
            let span = (hi - lo).max(1.);
            let step = w.max(h) * 0.35 / n.max(1.);
            field_wipe(
                t,
                style,
                move |x, y| {
                    let along = x * dx + y * dy;
                    let across = ((-x * dy + y * dx) - lo) / span;
                    let idx = (across * n).floor().clamp(0., n - 1.);
                    along + (idx - (n - 1.) / 2.).abs() * step
                },
                out,
            )
        }
        "circle_reveal" => {
            let (cx, cy) = centre(t)?;
            field_wipe(t, style, move |x, y| shape_dist(0, x - cx, y - cy), out)
        }
        "box_reveal" => {
            let (cx, cy) = centre(t)?;
            field_wipe(
                t,
                style,
                move |x, y| (x - cx).abs().max((y - cy).abs() * aspect),
                out,
            )
        }
        "diamond_reveal" => {
            let (cx, cy) = centre(t)?;
            field_wipe(
                t,
                style,
                move |x, y| (x - cx).abs() + (y - cy).abs() * aspect,
                out,
            )
        }
        "cross_reveal" => {
            let (cx, cy) = centre(t)?;
            field_wipe(
                t,
                style,
                move |x, y| {
                    let (u, v) = ((x - cx).abs(), (y - cy).abs() * aspect);
                    let k = 0.3;
                    (u / k).max(v).min(u.max(v / k))
                },
                out,
            )
        }
        "double_doors" => {
            let vertical = t.params.choice("orientation")? == 1;
            let (cx, cy) = (w / 2., h / 2.);
            field_wipe(
                t,
                style,
                move |x, y| {
                    if vertical {
                        (x - cx).abs()
                    } else {
                        (y - cy).abs() * aspect
                    }
                },
                out,
            )
        }
        "corner_inset" => {
            let (ox, oy) = match t.params.choice("corner")? {
                1 => (w, 0.),
                2 => (w, h),
                3 => (0., h),
                _ => (0., 0.),
            };
            field_wipe(
                t,
                style,
                move |x, y| (x - ox).abs().max((y - oy).abs() * aspect),
                out,
            )
        }
        _ => Err(Error::Invalid("transition kind")),
    }
}

fn stretch(t: &Tx, out: &mut Out) -> Result<()> {
    let choice = t.params.choice("direction")?;
    let (dx, dy): (f32, f32) = match choice {
        0 => (0., 1.),
        1 => (-1., 0.),
        2 => (0., -1.),
        _ => (1., 0.),
    };
    let (w, h) = (t.width as f32, t.height as f32);
    let length = px(t, "stretch")?.max(1.);
    let (lo, hi) = field_range(w, h, &|x: f32, y: f32| x * dx + y * dy, out.hooks())?;
    let edge = lo - length + t.progress * (hi - lo + 2. * length);
    out.paint(move |x, y| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let s = fx * dx + fy * dy;
        if s < edge {
            t.incoming_at(x, y)
        } else if s < edge + length {
            let back = s - edge;
            let smear = super::pixels::sample_clamped(t.incoming, fx - dx * back, fy - dy * back);
            mix(
                smear,
                t.outgoing_at(x, y),
                smoothstep(0., 1., back / length),
            )
        } else {
            t.outgoing_at(x, y)
        }
    })
}

fn neon_edge(t: &Tx, out: &mut Out) -> Result<()> {
    let (dx, dy) = angle_dir(angle_deg(t, "angle")?);
    let glow = px(t, "glow")?.max(1.);
    let col = t.params.color("color")?;
    let amount = t.params.fraction("amount")?;
    let (w, h) = (t.width as f32, t.height as f32);
    let (lo, hi) = field_range(w, h, &|x: f32, y: f32| x * dx + y * dy, out.hooks())?;
    let margin = glow * 0.5 + 2.;
    let thr = lo - margin + t.progress * (hi - lo + 2. * margin);
    let fade = (t.progress * 10.).min((1. - t.progress) * 10.).min(1.);
    let style = Style::new(0., 0., [0.; 4], 0.5 * t.scale.clamp(0.25, 1.))?;
    out.paint(move |x, y| {
        let d = (x as f32 + 0.5) * dx + (y as f32 + 0.5) * dy - thr;
        let base = wipe_px(&style, t.outgoing_at(x, y), t.incoming_at(x, y), d);
        let core = (-(d / (glow * 0.08)).powi(2)).exp();
        let halo = (-d.abs() / (glow * 0.5)).exp();
        let k = (core * 1.5 + halo * 0.8) * amount * fade;
        super::pixels::add_light(base, col, k)
    })
}

/// Smooth acceleration and deceleration, exactly 0 and 1 at the ends.
#[inline]
fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    if t < 0.5 {
        4. * t * t * t
    } else {
        1. - (-2. * t + 2.).powi(3) / 2.
    }
}

/// A grid of shapes sweeping in the catalogue order.
fn shape_dissolve(t: &Tx, out: &mut Out) -> Result<()> {
    let shape = t.params.choice("shape")?;
    let size = px(t, "size")?.max(4.);
    let rotation = angle_deg(t, "rotation")?;
    let order = t.params.choice("order")?;
    let seed = t.params.seed("seed")?;
    let style = Style::new(px(t, "softness")?, 0., [0.; 4], 1.)?;
    let (w, h) = (t.width as f32, t.height as f32);
    let (cols, rows) = ((w / size).ceil().max(1.), (h / size).ceil().max(1.));
    let diagonal = ((cols / 2.).powi(2) + (rows / 2.).powi(2)).sqrt().max(1.);
    field_wipe(
        t,
        style,
        move |x, y| {
            let (cx, cy) = ((x / size).floor(), (y / size).floor());
            let (lx, ly) = (x - (cx + 0.5) * size, y - (cy + 0.5) * size);
            let (u, v) = rot(lx / (size * 0.5), ly / (size * 0.5), rotation);
            let delay = match order {
                1 => cx / cols,
                2 => {
                    (((cx + 0.5) - cols / 2.).powi(2) + ((cy + 0.5) - rows / 2.).powi(2)).sqrt()
                        / diagonal
                }
                _ => hash(cx as i32, cy as i32, seed),
            };
            (shape_dist(shape, u, v) + delay * 1.5) * size * 0.5
        },
        out,
    )
}

/// One shape growing from the entry side to cover the frame.
fn shape_flow(t: &Tx, out: &mut Out) -> Result<()> {
    let shape = t.params.choice("shape")?;
    let (dx, dy) = match t.params.choice("direction")? {
        0 => (0., 1.),
        1 => (-1., 0.),
        2 => (0., -1.),
        _ => (1., 0.),
    };
    let style = Style::new(px(t, "softness")?, 0., [0.; 4], 1.)?;
    let (w, h) = (t.width as f32, t.height as f32);
    let (c1x, c1y) = (w / 2., h / 2.);
    // Start beside the frame, far enough out that the feather reaches no pixel at progress 0.
    let margin = style.margin();
    let c0x = c1x - dx * w * 0.75 - dx.signum() * margin;
    let c0y = c1y - dy * h * 0.75 - dy.signum() * margin;
    let (_, cover) = field_range(
        w,
        h,
        &|x: f32, y: f32| shape_dist(shape, x - c1x, y - c1y),
        out.hooks(),
    )?;
    let maximum_radius = cover.max(0.) * 1.05 + margin + 1.;
    let e = ease_out(t.progress);
    let (cx, cy) = (c0x + (c1x - c0x) * e, c0y + (c1y - c0y) * e);
    let radius = maximum_radius * ease_in_out(t.progress);
    out.paint(move |x, y| {
        let d = shape_dist(shape, x as f32 + 0.5 - cx, y as f32 + 0.5 - cy) - radius;
        wipe_px(&style, t.outgoing_at(x, y), t.incoming_at(x, y), d)
    })
}
