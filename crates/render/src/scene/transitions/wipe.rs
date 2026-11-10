//! Shape wipes driven by a scalar field: the incoming clip appears where `field < T(p)`.
use super::pixels::{FRAC_PI_2, PI, TAU, angle_deg, angle_dir, centre, mix, px, smoothstep};
use super::{Out, Tx};
use crate::{Error, Result};

/// Edge styling read from the shared wipe parameters.
#[derive(Clone, Copy, Debug)]
pub(super) struct Style {
    feather: f32,
    border: f32,
    border_color: [f32; 4],
    aa: f32,
}
impl Style {
    fn of(t: &Tx) -> Result<Self> {
        let aa = [0.5, 1., 1.75, 2.5][t.params.choice("antialias")?.min(3) as usize]
            * t.scale.clamp(0.25, 1.);
        Ok(Self {
            feather: px(t, "feather")?,
            border: px(t, "border_width")?,
            border_color: t.params.color("border_color")?,
            aa,
        })
    }
    /// The grain wipe's softer edge: two working pixels of feather and full anti-aliasing.
    pub(super) fn for_grain(t: &Tx, border: f32, border_color: [f32; 4]) -> Self {
        Self {
            feather: 2. * t.scale,
            border,
            border_color,
            aa: 1.,
        }
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

fn wipe_px(style: &Style, a: [f32; 4], b: [f32; 4], d: f32) -> [f32; 4] {
    let (kb, ko) = coverage(style, d);
    std::array::from_fn(|k| {
        a.get(k).copied().unwrap_or(0.) * (1. - ko)
            + style.border_color.get(k).copied().unwrap_or(0.) * (ko - kb)
            + b.get(k).copied().unwrap_or(0.) * kb
    })
}

/// Min/max of `field` over a coarse grid covering the frame.
fn field_range(w: f32, h: f32, field: &(impl Fn(f32, f32) -> f32 + Sync)) -> (f32, f32) {
    let n = 48;
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for j in 0..=n {
        for i in 0..=n {
            let v = field(w * i as f32 / n as f32, h * j as f32 / n as f32);
            if v.is_finite() {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
    }
    if lo.is_finite() { (lo, hi) } else { (0., 1.) }
}

/// The sweeping wipe: incoming where `field < T(p)`, field in pixels.
pub(super) fn field_wipe(
    t: &Tx,
    style: Style,
    field: impl Fn(f32, f32) -> f32 + Sync,
    out: &mut Out,
) -> Result<()> {
    let (w, h) = (t.width as f32, t.height as f32);
    let (lo, hi) = field_range(w, h, &field);
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
            let (lo, hi) = field_range(w, h, &|x: f32, y: f32| -x * dy + y * dx);
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
            field_wipe(
                t,
                style,
                move |x, y| ((x - cx).powi(2) + (y - cy).powi(2)).sqrt(),
                out,
            )
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
    let (lo, hi) = field_range(w, h, &|x: f32, y: f32| x * dx + y * dy);
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
    let (lo, hi) = field_range(w, h, &|x: f32, y: f32| x * dx + y * dy);
    let margin = glow * 0.5 + 2.;
    let thr = lo - margin + t.progress * (hi - lo + 2. * margin);
    let fade = (t.progress * 10.).min((1. - t.progress) * 10.).min(1.);
    let style = Style {
        feather: 0.,
        border: 0.,
        border_color: [0.; 4],
        aa: 0.5 * t.scale.clamp(0.25, 1.),
    };
    out.paint(move |x, y| {
        let d = (x as f32 + 0.5) * dx + (y as f32 + 0.5) * dy - thr;
        let base = wipe_px(&style, t.outgoing_at(x, y), t.incoming_at(x, y), d);
        let core = (-(d / (glow * 0.08)).powi(2)).exp();
        let halo = (-d.abs() / (glow * 0.5)).exp();
        let k = (core * 1.5 + halo * 0.8) * amount * fade;
        super::pixels::add_light(base, col, k)
    })
}
