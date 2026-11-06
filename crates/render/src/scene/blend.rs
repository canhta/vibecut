//! Separable and nonseparable blend functions.
use crate::{Error, ExecutionHooks, Image, Result, check};
pub use project::BlendMode;

/// Source-over on finite premultiplied linear RGBA in 0..=1, RGB <= alpha.
/// Opacity scales all source channels. No transfer conversion occurs here.
pub fn pixel(b: [f32; 4], s: [f32; 4], opacity: f32, mode: BlendMode) -> [f32; 4] {
    let [br, bg, bb, ba] = b;
    let [sr, sg, sb, sa] = s;
    let a = sa * opacity;
    if a <= 0.0 {
        return b;
    }
    if mode == BlendMode::Normal || ba <= 0.0 {
        let mut out = b;
        crate::image::over(&mut out, s, opacity);
        return out;
    }
    let mixed = blend_rgb(
        mode,
        [br / ba, bg / ba, bb / ba],
        [sr / sa, sg / sa, sb / sa],
    );
    let alpha = a + ba * (1.0 - a);
    let mut out = [0.0; 4];
    for (((o, b), s), m) in out
        .iter_mut()
        .take(3)
        .zip([br, bg, bb])
        .zip([sr, sg, sb])
        .zip(mixed)
    {
        *o = (s * opacity * (1.0 - ba) + b * (1.0 - a) + a * ba * m.clamp(0.0, 1.0))
            .clamp(0.0, alpha);
    }
    if let Some(v) = out.get_mut(3) {
        *v = alpha;
    }
    out
}

/// Same-size working images; validates opacity and geometry before mutation.
/// Coverage must already have scaled the source's four channels together.
pub fn composite(
    dst: &mut Image,
    src: &Image,
    opacity: f32,
    mode: BlendMode,
    hooks: &dyn ExecutionHooks,
) -> Result<()> {
    if dst.width != src.width
        || dst.height != src.height
        || !opacity.is_finite()
        || !(0.0..=1.0).contains(&opacity)
    {
        return Err(Error::Invalid("blend inputs"));
    }
    for (d, s) in dst
        .pixels
        .chunks_mut(dst.width as usize)
        .zip(src.pixels.chunks(src.width as usize))
    {
        check(hooks)?;
        for (d, s) in d.iter_mut().zip(s) {
            *d = pixel(*d, *s, opacity, mode);
        }
    }
    Ok(())
}

fn sep(mode: BlendMode, b: f32, s: f32) -> f32 {
    match mode {
        BlendMode::Darken => b.min(s),
        BlendMode::Multiply => b * s,
        BlendMode::ColorBurn => {
            if b >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - b) / s).min(1.0)
            }
        }
        BlendMode::Lighten => b.max(s),
        BlendMode::Screen => b + s - b * s,
        BlendMode::ColorDodge => {
            if b <= 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        }
        BlendMode::PlusLighter => (b + s).min(1.0),
        BlendMode::Overlay => sep(BlendMode::HardLight, s, b),
        BlendMode::HardLight => {
            if s <= 0.5 {
                b * 2.0 * s
            } else {
                sep(BlendMode::Screen, b, 2.0 * s - 1.0)
            }
        }
        BlendMode::SoftLight => {
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                let d = if b <= 0.25 {
                    ((16.0 * b - 12.0) * b + 4.0) * b
                } else {
                    b.sqrt()
                };
                b + (2.0 * s - 1.0) * (d - b)
            }
        }
        BlendMode::Difference => (b - s).abs(),
        BlendMode::Exclusion => b + s - 2.0 * b * s,
        _ => s,
    }
}
fn lum(c: [f32; 3]) -> f32 {
    let [r, g, b] = c;
    0.3 * r + 0.59 * g + 0.11 * b
}
fn clip_color(c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let [r, g, b] = c;
    let n = r.min(g).min(b);
    let x = r.max(g).max(b);
    let mut o = c;
    if n < 0.0 {
        for v in &mut o {
            *v = l + (*v - l) * l / (l - n).max(1e-6);
        }
    }
    if x > 1.0 {
        for v in &mut o {
            *v = l + (*v - l) * (1.0 - l) / (x - l).max(1e-6);
        }
    }
    o
}
fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color(c.map(|v| v + d))
}
fn sat(c: [f32; 3]) -> f32 {
    let [r, g, b] = c;
    r.max(g).max(b) - r.min(g).min(b)
}
fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let [r, g, b] = c;
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    if mx - mn <= 1e-6 {
        return [0.0; 3];
    }
    c.map(|v| (v - mn) * s / (mx - mn))
}
/// Blend straight colour components in the caller's working space.
pub fn blend_rgb(mode: BlendMode, b: [f32; 3], s: [f32; 3]) -> [f32; 3] {
    match mode {
        BlendMode::Hue => set_lum(set_sat(s, sat(b)), lum(b)),
        BlendMode::Saturation => set_lum(set_sat(b, sat(s)), lum(b)),
        BlendMode::Color => set_lum(s, lum(b)),
        BlendMode::Luminosity => set_lum(b, lum(s)),
        m => {
            let [br, bg, bb] = b;
            let [sr, sg, sb] = s;
            [sep(m, br, sr), sep(m, bg, sg), sep(m, bb, sb)]
        }
    }
}
