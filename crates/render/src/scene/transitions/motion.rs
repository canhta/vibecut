//! Moving-frame transitions over textured cards, direct frame samples and camera moves.
//!
//! Every kind composes textured cards under the pinhole camera in [`super::cards`] or samples the
//! two frames directly. Motion blur averages a bounded number of progress samples; the sample
//! count comes from the `motion_blur` parameter and never exceeds twelve.

use super::cards::{Card, V3, cross, eye, focal_for};
use super::pixels::{
    FRAC_PI_2, PI, TAU, bell, ease_in, ease_out, hash, line_blur, mid_mix, mix, over,
    pixel_clamped, px, sample_clamped, scale4, shutter, smoothstep, taps, temporal, zoom_blur,
};
use super::{Out, Tx};
use crate::image::Image;
use crate::{Error, ExecutionHooks, Result};

/// The output-size cap every card scene inherits from its destination image.
const OUTPUT_PIXELS: u64 = 1 << 26;

/// One textured card in a 3D scene.
#[derive(Clone, Copy)]
struct Layer<'a> {
    card: Card,
    front: &'a Image,
    /// Image on the back face (mirrored so it reads correctly after a half turn about the y axis
    /// (`mirror_y = false`) or the x axis (`mirror_y = true`)); `None` = one-sided.
    back: Option<&'a Image>,
    mirror_y: bool,
    /// Visible part of the card in image pixels (x0, y0, x1, y1).
    clip: [f32; 4],
    shade: bool,
    opacity: f32,
}

impl<'a> Layer<'a> {
    fn new(card: Card, front: &'a Image) -> Self {
        Layer {
            card,
            front,
            back: None,
            mirror_y: false,
            clip: [0., 0., front.width() as f32, front.height() as f32],
            shade: false,
            opacity: 1.,
        }
    }
    fn with_back(mut self, back: &'a Image, mirror_y: bool) -> Self {
        self.back = Some(back);
        self.mirror_y = mirror_y;
        self
    }
    fn clipped(mut self, clip: [f32; 4]) -> Self {
        self.clip = clip;
        self
    }
    fn shaded(mut self) -> Self {
        self.shade = true;
        self
    }
}

/// Composite the layers seen through screen pixel (x, y), nearest last.
fn composite(layers: &[Layer], eye: V3, x: f32, y: f32) -> [f32; 4] {
    const MAX: usize = 8;
    let mut hits = [(0.0f32, [0.0f32; 4]); MAX];
    let mut count = 0;
    for layer in layers {
        let Some((lx, ly, depth, front)) = layer.card.hit(eye, x, y) else {
            continue;
        };
        let [x0, y0, x1, y1] = layer.clip;
        let edge = (lx - x0).min(x1 - lx).min(ly - y0).min(y1 - ly);
        let coverage = (edge + 0.5).clamp(0., 1.);
        if coverage <= 0. {
            continue;
        }
        let (image, sx, sy) = if front {
            (layer.front, lx, ly)
        } else if let Some(back) = layer.back {
            if layer.mirror_y {
                (back, lx, back.height() as f32 - ly)
            } else {
                (back, back.width() as f32 - lx, ly)
            }
        } else {
            continue;
        };
        let mut pixel = sample_clamped(image, sx, sy);
        let mut opacity = coverage * layer.opacity;
        if layer.shade {
            let normal = cross(layer.card.ex, layer.card.ey);
            let [nx, ny, nz] = normal;
            let length = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-9);
            let facing = (nz / length).abs();
            let light = 0.45 + 0.55 * facing;
            let [r, g, b, a] = pixel;
            pixel = [r * light, g * light, b * light, a];
        }
        opacity = opacity.clamp(0., 1.);
        if let Some(slot) = hits.get_mut(count) {
            *slot = (depth, scale4(pixel, opacity));
            count += 1;
        }
    }
    if let Some(front_to_back) = hits.get_mut(..count) {
        front_to_back.sort_by(|a, b| b.0.total_cmp(&a.0));
    }
    hits.get(..count)
        .unwrap_or(&[])
        .iter()
        .fold([0.; 4], |accumulated, &(_, color)| over(accumulated, color))
}

/// Render one card scene, averaged over the motion-blur time samples.
fn scene_into<'a>(
    t: &Tx<'a>,
    out: &mut Out<'_>,
    focal: f32,
    blur: (f32, usize),
    build: impl Fn(f32) -> Vec<Layer<'a>> + Sync,
) -> Result<()> {
    let (shutter, samples) = blur;
    let times: Vec<f32> = if samples <= 1 || shutter <= 0. {
        vec![t.progress]
    } else {
        (0..samples)
            .map(|i| {
                let offset = i as f32 / (samples - 1) as f32 - 0.5;
                (t.progress + offset * shutter).clamp(0., 1.)
            })
            .collect()
    };
    let scenes: Vec<Vec<Layer<'a>>> = times.iter().map(|&q| build(q)).collect();
    let camera = eye(t.width as f32, t.height as f32, focal);
    let inverse = 1. / scenes.len().max(1) as f32;
    out.paint(|x, y| {
        let mut accumulated = [0.; 4];
        for scene in &scenes {
            let color = composite(scene, camera, x as f32 + 0.5, y as f32 + 0.5);
            accumulated = std::array::from_fn(|k| {
                accumulated.get(k).copied().unwrap_or(0.) + color.get(k).copied().unwrap_or(0.)
            });
        }
        std::array::from_fn(|k| accumulated.get(k).copied().unwrap_or(0.) * inverse)
    })
}

/// Render one card scene into a new image (for a kind that post-processes it).
fn render<'a>(
    t: &Tx<'a>,
    hooks: &dyn ExecutionHooks,
    focal: f32,
    blur: (f32, usize),
    build: impl Fn(f32) -> Vec<Layer<'a>> + Sync,
) -> Result<Image> {
    let mut image = Image::new(t.outgoing.width(), t.outgoing.height(), [0.; 4], hooks)?;
    {
        let mut out = Out {
            pixels: &mut image.pixels,
            width: t.width,
            hooks,
        };
        scene_into(t, &mut out, focal, blur, build)?;
    }
    Ok(image)
}

fn center3(t: &Tx) -> V3 {
    [t.width as f32 / 2., t.height as f32 / 2., 0.]
}

fn dir(t: &Tx) -> Result<(f32, f32)> {
    Ok(match t.params.choice("direction")? {
        0 => (0., 1.),
        1 => (-1., 0.),
        2 => (0., -1.),
        _ => (1., 0.),
    })
}

fn travel(t: &Tx, dx: f32, dy: f32) -> f32 {
    (dx.abs() * t.width as f32 + dy.abs() * t.height as f32).max(1.)
}

#[inline]
fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    if t < 0.5 {
        4. * t * t * t
    } else {
        1. - (-2. * t + 2.).powi(3) / 2.
    }
}

/// "Back-out" easing with overshoot `s` (0 = plain ease-out).
fn back_out(x: f32, s: f32) -> f32 {
    let x = x.clamp(0., 1.) - 1.;
    1. + (s + 1.) * x * x * x + s * x * x
}

/// Damped spring from 0 to exactly 1.
fn spring(x: f32, bounce: f32) -> f32 {
    let w = (2. + 4. * bounce) * PI;
    let k = 4. + 6. * (1. - bounce);
    let f = |u: f32| 1. - (-k * u).exp() * (w * u).cos();
    // A slower start (u = x^1.6) so the move reads before the bounce.
    let x = x.clamp(0., 1.).powf(1.6);
    f(x) + x * (1. - f(1.))
}

/// Refuse a frame whose card scene cannot be bounded by the output-size cap.
fn check_geometry(width: usize, height: usize) -> Result<()> {
    if width == 0
        || height == 0
        || (width as u64)
            .checked_mul(height as u64)
            .is_none_or(|pixels| pixels > OUTPUT_PIXELS)
    {
        return Err(Error::Invalid("transition frame size"));
    }
    Ok(())
}

/// Dispatch one moving-frame kind.
pub(super) fn run(id: &str, t: &Tx, out: &mut Out) -> Result<()> {
    check_geometry(t.width, t.height)?;
    match id {
        "push" => push(t, out),
        "slide" => slide(t, out),
        "roll" => roll(t, out),
        "split" => split(t, out),
        "whip" => whip(t, out),
        "stretch" => stretch(t, out),
        "film_roll" => film_roll(t, out),
        "roll_3d" => roll_3d(t, out),
        "block_motion" => block_motion(t, out),
        "flip_motion" => flip(t, out, true),
        "spin_3d" => flip(t, out, false),
        "fold_motion" => fold_motion(t, out),
        "pop_motion" => pop_motion(t, out),
        "pull_motion" => pull_motion(t, out),
        "spin_motion" => spin_motion(t, out),
        "spring_motion" => spring_motion(t, out),
        "travel_motion" => travel_motion(t, out),
        "spinback_3d" => spinback_3d(t, out),
        "frame" => frame(t, out),
        "louver" => louver(t, out),
        "mirror_transition" => mirror_transition(t, out),
        "page_peel" => page_peel(t, out),
        "slice" => slice(t, out),
        "wave" => wave(t, out),
        "motion_camera" => motion_camera(t, out),
        "motion_tween" => motion_tween(t, out),
        _ => Err(Error::Invalid("transition kind")),
    }
}

fn push(t: &Tx, out: &mut Out) -> Result<()> {
    let (dx, dy) = dir(t)?;
    let travel = travel(t, dx, dy);
    let far = focal_for(t.width as f32, t.height as f32, 0.);
    scene_into(t, out, far, shutter(t, 0.12)?, move |q| {
        let e = ease_in_out(q);
        vec![
            Layer::new(
                Card::flat().translate([dx * travel * e, dy * travel * e, 0.]),
                t.outgoing,
            ),
            Layer::new(
                Card::flat().translate([dx * travel * (e - 1.), dy * travel * (e - 1.), 0.]),
                t.incoming,
            ),
        ]
    })
}

fn slide(t: &Tx, out: &mut Out) -> Result<()> {
    let (dx, dy) = dir(t)?;
    let travel = travel(t, dx, dy);
    let far = focal_for(t.width as f32, t.height as f32, 0.);
    scene_into(t, out, far, shutter(t, 0.12)?, move |q| {
        let e = ease_in_out(q);
        vec![
            Layer::new(Card::flat(), t.outgoing),
            Layer::new(
                Card::flat().translate([dx * travel * (e - 1.), dy * travel * (e - 1.), 0.]),
                t.incoming,
            ),
        ]
    })
}

fn roll(t: &Tx, out: &mut Out) -> Result<()> {
    let (dx, dy) = dir(t)?;
    let (w, h) = (t.width as f32, t.height as f32);
    let horizontal = dx != 0.;
    // A and B ride a big wheel whose hub sits below (or beside) the frame.
    let radius = if horizontal { h * 2. } else { w * 2. };
    let hub: V3 = if horizontal {
        [w / 2., h / 2. + radius, 0.]
    } else {
        [w / 2. - radius, h / 2., 0.]
    };
    let gap = (if horizontal { w } else { h }) * 1.15 / radius;
    let sign = if horizontal { dx.signum() } else { dy.signum() };
    let far = focal_for(w, h, 0.);
    scene_into(t, out, far, shutter(t, 0.1)?, move |q| {
        let angle = ease_in_out(q) * gap * sign;
        vec![
            Layer::new(Card::flat().rot_z(hub, angle), t.outgoing),
            Layer::new(Card::flat().rot_z(hub, angle - gap * sign), t.incoming),
        ]
    })
}

fn split(t: &Tx, out: &mut Out) -> Result<()> {
    let (w, h) = (t.width as f32, t.height as f32);
    let vertical_cut = t.params.choice("orientation")? == 0;
    let far = focal_for(w, h, 0.);
    scene_into(t, out, far, shutter(t, 0.12)?, move |q| {
        let e = ease_in_out(q);
        // A shrinking one-pixel overlap keeps the outgoing frame whole near p = 0 without
        // leaving an incoming column at the far edge near p = 1.
        let overlap = 1. - e;
        let (left, right) = if vertical_cut {
            ([0., 0., w / 2. + overlap, h], [w / 2. - overlap, 0., w, h])
        } else {
            ([0., 0., w, h / 2. + overlap], [0., h / 2. - overlap, w, h])
        };
        let (ox, oy) = if vertical_cut {
            (w / 2. * e, 0.)
        } else {
            (0., h / 2. * e)
        };
        vec![
            Layer::new(Card::flat(), t.incoming),
            Layer::new(Card::flat().translate([-ox, -oy, -0.01]), t.outgoing).clipped(left),
            Layer::new(Card::flat().translate([ox, oy, -0.01]), t.outgoing).clipped(right),
        ]
    })
}

fn whip(t: &Tx, out: &mut Out) -> Result<()> {
    let (dx, dy) = dir(t)?;
    let travel = travel(t, dx, dy);
    let far = focal_for(t.width as f32, t.height as f32, 0.);
    let progress = t.progress;
    let e = {
        // Fast in the middle: a steep sigmoid normalised to 0..1.
        let s = |u: f32| 1. / (1. + (-(u - 0.5) * 14.).exp());
        (s(progress) - s(0.)) / (s(1.) - s(0.))
    };
    let pushed = render(t, out.hooks(), far, (0., 1), move |_| {
        vec![
            Layer::new(
                Card::flat().translate([dx * travel * e, dy * travel * e, 0.]),
                t.outgoing,
            ),
            Layer::new(
                Card::flat().translate([dx * travel * (e - 1.), dy * travel * (e - 1.), 0.]),
                t.incoming,
            ),
        ]
    })?;
    let length = t.params.fraction("blur")? * bell(progress) * travel * 0.18;
    if length < 0.75 {
        out.paint(|x, y| sample_clamped(&pushed, x as f32 + 0.5, y as f32 + 0.5))
    } else {
        let n = taps(length * 2.);
        out.paint(|x, y| {
            line_blur(
                &pushed,
                x as f32 + 0.5,
                y as f32 + 0.5,
                dx * length,
                dy * length,
                n,
            )
        })
    }
}

fn stretch(t: &Tx, out: &mut Out) -> Result<()> {
    let (dx, dy) = dir(t)?;
    let (w, h) = (t.width as f32, t.height as f32);
    let horizontal = dx != 0.;
    let progress = t.progress;
    let e = ease_in_out(progress);
    // B grows from the entry side while A squeezes towards the exit side.
    let (length, axis_x) = if horizontal { (w, true) } else { (h, false) };
    let forward = if horizontal { dx > 0. } else { dy > 0. };
    let split = if forward {
        length * e
    } else {
        length * (1. - e)
    };
    out.paint(|xi, yi| {
        let x = xi as f32 + 0.5;
        let y = yi as f32 + 0.5;
        let position = if axis_x { x } else { y };
        let (incoming, local) = if forward {
            if position < split {
                (true, position / split.max(1e-3) * length)
            } else {
                (
                    false,
                    (position - split) / (length - split).max(1e-3) * length,
                )
            }
        } else if position >= split {
            (
                true,
                (position - split) / (length - split).max(1e-3) * length,
            )
        } else {
            (false, position / split.max(1e-3) * length)
        };
        let (sx, sy) = if axis_x { (local, y) } else { (x, local) };
        if incoming {
            sample_clamped(t.incoming, sx, sy)
        } else {
            sample_clamped(t.outgoing, sx, sy)
        }
    })
}

fn film_roll(t: &Tx, out: &mut Out) -> Result<()> {
    let (dx, dy) = dir(t)?;
    let (w, h) = (t.width as f32, t.height as f32);
    let horizontal = dx != 0.;
    let travel = travel(t, dx, dy);
    let gap = px(t, "gap")?;
    let gap_color = t.params.color("gap_color")?;
    let step = travel + gap;
    let (shutter, samples) = shutter(t, 0.1)?;
    let progress = t.progress;
    out.paint(|xi, yi| {
        let x = xi as f32 + 0.5;
        let y = yi as f32 + 0.5;
        temporal(x, y, progress, shutter, samples, &|x, y, q| {
            let offset = step * ease_in_out(q);
            let (ax, ay) = (x - dx * offset, y - dy * offset);
            let inside = |u: f32, v: f32| u >= 0. && v >= 0. && u < w && v < h;
            if inside(ax, ay) {
                return pixel_clamped(t.outgoing, ax, ay);
            }
            let (bx, by) = (ax + dx * step, ay + dy * step);
            if inside(bx, by) {
                return pixel_clamped(t.incoming, bx, by);
            }
            // The gap between the frames, with sprocket holes along both edges.
            let across = if horizontal { y } else { x };
            let across_length = if horizontal { h } else { w };
            let along = if horizontal { ax } else { ay };
            let edge_band = (across / across_length < 0.12) || (across / across_length > 0.88);
            let pitch = across_length / 14.;
            let f = (across / pitch).fract();
            let g = ((along.rem_euclid(step) - travel) / gap.max(1.)).clamp(0., 1.);
            let hole = !edge_band && (0.25..0.75).contains(&f) && (0.3..0.7).contains(&g);
            if hole { [0.5, 0.5, 0.5, 1.] } else { gap_color }
        })
    })
}

fn roll_3d(t: &Tx, out: &mut Out) -> Result<()> {
    let (dx, dy) = dir(t)?;
    let (w, h) = (t.width as f32, t.height as f32);
    let horizontal = dx != 0.;
    // A cube: A on the front face, B on the face that turns to the front.
    let side = if horizontal { w } else { h };
    let center: V3 = [w / 2., h / 2., side / 2.];
    let focal = focal_for(w, h, t.params.fraction("perspective")?);
    let sign = if horizontal {
        dx.signum()
    } else {
        -dy.signum()
    };
    scene_into(t, out, focal, (0., 1), move |q| {
        let angle = ease_in_out(q) * FRAC_PI_2 * sign;
        let back = side * 0.45 * bell(q);
        let rotate = |card: Card, angle: f32| {
            if horizontal {
                card.rot_y(center, angle)
            } else {
                card.rot_x(center, angle)
            }
        };
        vec![
            Layer::new(
                rotate(Card::flat(), angle).translate([0., 0., back]),
                t.outgoing,
            )
            .shaded(),
            Layer::new(
                rotate(rotate(Card::flat(), -FRAC_PI_2 * sign), angle).translate([0., 0., back]),
                t.incoming,
            )
            .shaded(),
        ]
    })
}

fn block_motion(t: &Tx, out: &mut Out) -> Result<()> {
    let (dx, dy) = dir(t)?;
    let (w, h) = (t.width as f32, t.height as f32);
    let horizontal = dx != 0.;
    let cols = t.params.num("blocks")?.round().clamp(2., 32.);
    let rows = (cols * h / w).round().clamp(1., 32.);
    let (lanes, cells) = if horizontal {
        (rows, cols)
    } else {
        (cols, rows)
    };
    let (shutter, samples) = shutter(t, 0.08)?;
    let samples = samples.min(4);
    let progress = t.progress;
    let seed = 17;
    out.paint(|xi, yi| {
        let x = xi as f32 + 0.5;
        let y = yi as f32 + 0.5;
        temporal(x, y, progress, shutter, samples, &|x, y, q| {
            let (along, across, length_along, length_across) = if horizontal {
                (x, y, w, h)
            } else {
                (y, x, h, w)
            };
            let sign = if horizontal { dx } else { dy };
            let lane = (across / length_across * lanes)
                .floor()
                .clamp(0., lanes - 1.);
            let cell_length = length_along / cells;
            // Each destination tile keeps its own delay, so a pixel always belongs to exactly
            // one tile; sampling the source outside the frame leaves a gap between tiles.
            let cell = (along / cell_length).floor().clamp(0., cells - 1.);
            // Leading blocks (towards the exit side) leave first.
            let order = if sign > 0. { cells - 1. - cell } else { cell } / cells;
            let delay = 0.6 * (0.6 * order + 0.4 * hash(cell as i32, lane as i32, seed));
            let qi = ((q - delay) / (1. - 0.6)).clamp(0., 1.);
            let e = ease_in_out(qi);
            let source = |offset: f32| {
                let pa = along - offset;
                (pa >= 0. && pa < length_along).then_some(pa)
            };
            let a = source(sign * length_along * e).map(|pa| {
                let (sx, sy) = if horizontal { (pa, y) } else { (x, pa) };
                pixel_clamped(t.outgoing, sx, sy)
            });
            let b = source(sign * length_along * (e - 1.)).map(|pa| {
                let (sx, sy) = if horizontal { (pa, y) } else { (x, pa) };
                pixel_clamped(t.incoming, sx, sy)
            });
            match (a, b) {
                (Some(a), Some(b)) => over(b, a),
                (Some(a), None) => a,
                (None, Some(b)) => b,
                (None, None) => [0.; 4],
            }
        })
    })
}

fn flip(t: &Tx, out: &mut Out, flip: bool) -> Result<()> {
    let (w, h) = (t.width as f32, t.height as f32);
    let (dx, dy) = dir(t)?;
    let horizontal = dx != 0.;
    let focal = focal_for(w, h, t.params.fraction("perspective")?);
    let sign = if horizontal {
        dx.signum()
    } else {
        -dy.signum()
    };
    let blur = if flip { shutter(t, 0.06)? } else { (0., 1) };
    let center = center3(t);
    let size = w.max(h);
    scene_into(t, out, focal, blur, move |q| {
        let (e, back) = if flip {
            // Ease in, then settle with a small overshoot.
            (
                if q < 0.5 {
                    ease_in(q * 2.) * 0.5
                } else {
                    0.5 + back_out(q * 2. - 1., 1.2) * 0.5
                },
                0.25 * bell(q),
            )
        } else {
            (ease_in_out(q), 0.8 * bell(q))
        };
        let angle = e * PI * sign;
        let card = if horizontal {
            Card::flat().rot_y(center, angle)
        } else {
            Card::flat().rot_x(center, angle)
        };
        let card = card.translate([0., 0., back * size]);
        vec![
            Layer::new(card, t.outgoing)
                .with_back(t.incoming, !horizontal)
                .shaded(),
        ]
    })
}

fn fold_motion(t: &Tx, out: &mut Out) -> Result<()> {
    let (w, h) = (t.width as f32, t.height as f32);
    let (dx, dy) = dir(t)?;
    let horizontal = dx != 0.;
    let focal = focal_for(w, h, t.params.fraction("perspective")?);
    let sign = if dx > 0. || dy > 0. { 1. } else { -1. };
    let (entry, mid_axis, exit_axis): ([f32; 4], V3, V3) = match (horizontal, sign > 0.) {
        (true, true) => ([0., 0., w / 2. + 1., h], [w / 2., 0., 0.], [w, 0., 0.]),
        (true, false) => ([w / 2. - 1., 0., w, h], [w / 2., 0., 0.], [0., 0., 0.]),
        (false, true) => ([0., 0., w, h / 2. + 1.], [0., h / 2., 0.], [0., h, 0.]),
        (false, false) => ([0., h / 2. - 1., w, h], [0., h / 2., 0.], [0., 0., 0.]),
    };
    let exit = match (horizontal, sign > 0.) {
        (true, true) => [w / 2. - 1., 0., w, h],
        (true, false) => [0., 0., w / 2. + 1., h],
        (false, true) => [0., h / 2. - 1., w, h],
        (false, false) => [0., 0., w, h / 2. + 1.],
    };
    scene_into(t, out, focal, (0., 1), move |q| {
        let phi = PI * ease_in_out((q * 2.).min(1.)) * sign;
        let psi = FRAC_PI_2 * ease_in(((q - 0.5) * 2.).clamp(0., 1.)) * sign;
        let rotate = |card: Card, axis: V3, angle: f32| {
            if horizontal {
                card.rot_y(axis, angle)
            } else {
                card.rot_x(axis, -angle)
            }
        };
        let flap = rotate(
            rotate(Card::flat().translate([0., 0., -0.5]), mid_axis, phi),
            exit_axis,
            psi,
        );
        let base = rotate(Card::flat(), exit_axis, psi);
        // The outgoing side is edge-on by q = 1; fade its last sliver so the frame reaches the
        // incoming endpoint exactly instead of leaving a one-pixel card line.
        let fade = 1. - smoothstep(0.9, 1., q);
        let mut base = Layer::new(base, t.outgoing).clipped(exit).shaded();
        base.opacity = fade;
        let mut flap = Layer::new(flap, t.outgoing)
            .with_back(t.outgoing, !horizontal)
            .clipped(entry)
            .shaded();
        flap.opacity = fade;
        vec![Layer::new(Card::flat(), t.incoming), base, flap]
    })
}

fn pop_motion(t: &Tx, out: &mut Out) -> Result<()> {
    let (w, h) = (t.width as f32, t.height as f32);
    let overshoot = t.params.fraction("overshoot")? * 4.;
    let far = focal_for(w, h, 0.);
    let center = center3(t);
    scene_into(t, out, far, shutter(t, 0.05)?, move |q| {
        let scale_a = 1. - 0.3 * ease_in((q / 0.6).min(1.));
        let qb = ((q - 0.15) / 0.85).clamp(0., 1.);
        let scale_b = back_out(qb, overshoot).max(0.);
        let mut layer_a = Layer::new(Card::flat().scale_about(center, scale_a), t.outgoing);
        layer_a.opacity = 1. - smoothstep(0.35, 0.8, q);
        let mut layer_b = Layer::new(
            Card::flat()
                .scale_about(center, scale_b.max(1e-3))
                .translate([0., 0., -0.01]),
            t.incoming,
        );
        layer_b.opacity = smoothstep(0.1, 0.3, q);
        vec![layer_a, layer_b]
    })
}

fn pull_motion(t: &Tx, out: &mut Out) -> Result<()> {
    let (dx, dy) = dir(t)?;
    let travel = travel(t, dx, dy);
    let far = focal_for(t.width as f32, t.height as f32, 0.);
    let center = center3(t);
    scene_into(t, out, far, shutter(t, 0.1)?, move |q| {
        let e = ease_in_out(q);
        let s = 1. - 0.25 * bell(q);
        vec![
            Layer::new(
                Card::flat().scale_about(center, s).translate([
                    dx * travel * e,
                    dy * travel * e,
                    0.,
                ]),
                t.outgoing,
            ),
            Layer::new(
                Card::flat().scale_about(center, s).translate([
                    dx * travel * (e - 1.),
                    dy * travel * (e - 1.),
                    0.,
                ]),
                t.incoming,
            ),
        ]
    })
}

fn spin_motion(t: &Tx, out: &mut Out) -> Result<()> {
    let (w, h) = (t.width as f32, t.height as f32);
    let turn = t.params.num("rotation")?.to_radians()
        * if t.params.choice("spin")? == 1 {
            -1.
        } else {
            1.
        };
    let far = focal_for(w, h, 0.);
    let center = center3(t);
    scene_into(t, out, far, shutter(t, 0.06)?, move |q| {
        if q < 0.5 {
            let k = ease_in(q * 2.);
            vec![Layer::new(
                Card::flat()
                    .rot_z(center, turn * 0.5 * k)
                    .scale_about(center, (1. - k).max(1e-3)),
                t.outgoing,
            )]
        } else {
            let k = ease_out((q - 0.5) * 2.);
            vec![Layer::new(
                Card::flat()
                    .rot_z(center, -turn * 0.5 * (1. - k))
                    .scale_about(center, k.max(1e-3)),
                t.incoming,
            )]
        }
    })
}

fn spring_motion(t: &Tx, out: &mut Out) -> Result<()> {
    let (dx, dy) = dir(t)?;
    let travel = travel(t, dx, dy);
    let bounce = t.params.fraction("bounce")?;
    let far = focal_for(t.width as f32, t.height as f32, 0.);
    scene_into(t, out, far, shutter(t, 0.08)?, move |q| {
        let e = spring(q, bounce);
        vec![
            Layer::new(
                Card::flat().translate([dx * travel * e, dy * travel * e, 0.]),
                t.outgoing,
            ),
            Layer::new(
                Card::flat().translate([dx * travel * (e - 1.), dy * travel * (e - 1.), 0.]),
                t.incoming,
            ),
        ]
    })
}

fn travel_motion(t: &Tx, out: &mut Out) -> Result<()> {
    let (dx, dy) = dir(t)?;
    let travel = travel(t, dx, dy);
    let zoom = t.params.fraction("zoom")?;
    let far = focal_for(t.width as f32, t.height as f32, 0.);
    let center = center3(t);
    let step = travel * 1.08;
    scene_into(t, out, far, shutter(t, 0.08)?, move |q| {
        let s = 1. - zoom * bell(q);
        let pan = step * ease_in_out(q);
        vec![
            Layer::new(
                Card::flat()
                    .translate([dx * pan, dy * pan, 0.])
                    .scale_about(center, s),
                t.outgoing,
            ),
            Layer::new(
                Card::flat()
                    .translate([dx * (pan - step), dy * (pan - step), 0.])
                    .scale_about(center, s),
                t.incoming,
            ),
        ]
    })
}

fn spinback_3d(t: &Tx, out: &mut Out) -> Result<()> {
    let (w, h) = (t.width as f32, t.height as f32);
    let (dx, dy) = dir(t)?;
    let horizontal = dx != 0.;
    let depth = t.params.fraction("depth")?;
    let focal = focal_for(w, h, 0.6);
    let sign = if horizontal {
        dx.signum()
    } else {
        -dy.signum()
    };
    let center = center3(t);
    let size = w.max(h);
    scene_into(t, out, focal, (0., 1), move |q| {
        let z = depth * size * 1.5 * bell(q);
        if q < 0.5 {
            let angle = ease_in(q * 2.) * FRAC_PI_2 * sign;
            let card = if horizontal {
                Card::flat().rot_x(center, angle)
            } else {
                Card::flat().rot_y(center, angle)
            };
            vec![Layer::new(card.translate([0., 0., z]), t.outgoing).shaded()]
        } else {
            let angle = -(1. - ease_out((q - 0.5) * 2.)) * FRAC_PI_2 * sign;
            let card = if horizontal {
                Card::flat().rot_x(center, angle)
            } else {
                Card::flat().rot_y(center, angle)
            };
            vec![Layer::new(card.translate([0., 0., z]), t.incoming).shaded()]
        }
    })
}

fn frame(t: &Tx, out: &mut Out) -> Result<()> {
    let (w, h) = (t.width as f32, t.height as f32);
    // The catalogue has no direction parameter for this kind; the reference falls back to its
    // first choice, so the frame travels vertically.
    let (dx, dy) = (0., 1.);
    let travel = travel(t, dx, dy);
    let frame_width = px(t, "frame_width")?;
    let frame_color = t.params.color("frame_color")?;
    let minimum_scale = t.params.fraction("scale")?.clamp(0.05, 1.);
    let progress = t.progress;
    // Phase: shrink into a frame, travel across, grow out of the frame.
    let shrink = smoothstep(0., 0.3, progress) * (1. - smoothstep(0.7, 1., progress));
    let scale = 1. - (1. - minimum_scale) * shrink;
    let move_t = ease_in_out(((progress - 0.25) / 0.5).clamp(0., 1.));
    let border = frame_width * shrink;
    // Just far enough that the off-stage frame (and its border) is fully out of view.
    let step = travel * 0.5 * (1. + scale) + border * 2. + 2.;
    let layers = [
        (t.outgoing, move_t * step),
        (t.incoming, (move_t - 1.) * step),
    ];
    out.paint(|xi, yi| {
        let x = xi as f32 + 0.5;
        let y = yi as f32 + 0.5;
        let mut result = [0.; 4];
        for (image, offset) in layers {
            // Inverse of: scale about centre, then translate along the direction.
            let (ux, uy) = (x - dx * offset, y - dy * offset);
            let lx = (ux - w / 2.) / scale + w / 2.;
            let ly = (uy - h / 2.) / scale + h / 2.;
            let border_px = border / scale;
            let inside = lx >= 0. && lx < w && ly >= 0. && ly < h;
            let in_frame =
                lx >= -border_px && lx < w + border_px && ly >= -border_px && ly < h + border_px;
            let color = if inside {
                sample_clamped(image, lx, ly)
            } else if in_frame && border_px > 0. {
                frame_color
            } else {
                continue;
            };
            result = over(result, color);
        }
        result
    })
}

fn louver(t: &Tx, out: &mut Out) -> Result<()> {
    let (w, h) = (t.width as f32, t.height as f32);
    let n = t.params.num("slats")?.round().clamp(1., 64.);
    let horizontal_slats = t.params.choice("orientation")? == 0;
    let progress = t.progress;
    out.paint(|xi, yi| {
        let x = xi as f32 + 0.5;
        let y = yi as f32 + 0.5;
        let (position, length) = if horizontal_slats { (y, h) } else { (x, w) };
        let slat = length / n;
        let i = (position / slat).floor().clamp(0., n - 1.);
        let middle = (i + 0.5) * slat;
        let stagger = 0.4 * i / n.max(1.);
        let q = ((progress - stagger) / 0.6).clamp(0., 1.);
        let theta = ease_in_out(q) * PI;
        let cosine = theta.cos();
        if cosine.abs() < 1e-3 {
            return [0., 0., 0., 1.];
        }
        let d = position - middle;
        let local = d / cosine; // A (front) for cos > 0, B (back, re-flipped) for cos < 0
        if local.abs() > slat / 2. {
            return [0., 0., 0., 1.];
        }
        let source = middle + if cosine > 0. { local } else { -local };
        let (sx, sy) = if horizontal_slats {
            (x, source)
        } else {
            (source, y)
        };
        let color = if cosine > 0. {
            pixel_clamped(t.outgoing, sx, sy)
        } else {
            pixel_clamped(t.incoming, sx, sy)
        };
        let light = 0.5 + 0.5 * cosine.abs();
        let [r, g, b, a] = color;
        [r * light, g * light, b * light, a]
    })
}

fn mirror_transition(t: &Tx, out: &mut Out) -> Result<()> {
    let (dx, dy) = dir(t)?;
    let (w, h) = (t.width as f32, t.height as f32);
    let horizontal = dx != 0.;
    let length = travel(t, dx, dy);
    let (low, high) = (0., length);
    let reflection = length * 0.3;
    let edge = low - reflection + ease_in_out(t.progress) * (high - low + 2. * reflection);
    let negative = dx + dy < 0.;
    out.paint(|xi, yi| {
        let x = xi as f32 + 0.5;
        let y = yi as f32 + 0.5;
        // Along-axis coordinate measured from the entry side.
        let s = if horizontal {
            if negative { w - x } else { x }
        } else if negative {
            h - y
        } else {
            y
        };
        if s < edge {
            return pixel_clamped(t.incoming, x, y);
        }
        let outgoing = pixel_clamped(t.outgoing, x, y);
        let d = s - edge;
        if d < reflection {
            let mirror = edge - d; // Reflection of s about the moving line.
            let (sx, sy) = if horizontal {
                (if negative { w - mirror } else { mirror }, y)
            } else {
                (x, if negative { h - mirror } else { mirror })
            };
            let reflected = sample_clamped(t.incoming, sx, sy);
            return mix(outgoing, reflected, 0.75 * (1. - d / reflection));
        }
        outgoing
    })
}

fn page_peel(t: &Tx, out: &mut Out) -> Result<()> {
    let (w, h) = (t.width as f32, t.height as f32);
    let radius = px(t, "radius")?.max(1.);
    let shadow = t.params.flag("shadow")?;
    let (ox, oy) = match t.params.choice("corner")? {
        0 => (0., 0.),
        1 => (w, 0.),
        3 => (0., h),
        _ => (w, h),
    };
    let (mut nx, mut ny) = (w / 2. - ox, h / 2. - oy);
    let normal_length = (nx * nx + ny * ny).sqrt().max(1e-3);
    nx /= normal_length;
    ny /= normal_length;
    let smax = (w - 2. * ox).abs() * nx.abs() + (h - 2. * oy).abs() * ny.abs();
    let progress = t.progress;
    let f = -1. + progress * (smax + radius * 1.2 + 2.);
    let inside = |px: f32, py: f32| px >= 0. && py >= 0. && px < w && py < h;
    let washed = |c: [f32; 4], light: f32| {
        let alpha = c.get(3).copied().unwrap_or(0.);
        let mixed = mix(c, [alpha, alpha, alpha, alpha], 0.6);
        let [r, g, b, a] = mixed;
        [r * light, g * light, b * light, a]
    };
    out.paint(|xi, yi| {
        let x = xi as f32 + 0.5;
        let y = yi as f32 + 0.5;
        let s = (x - ox) * nx + (y - oy) * ny;
        let page_at = |so: f32| (x - nx * (s - so), y - ny * (s - so));
        // 1. The flipped flat part (height 2r) over the unpeeled page.
        if s >= f {
            let so = 2. * f - s - PI * radius;
            let (px, py) = page_at(so);
            if inside(px, py) {
                return washed(pixel_clamped(t.outgoing, px, py), 0.95);
            }
        }
        let d = f - s;
        if (0. ..radius).contains(&d) {
            // 2. Upper half of the cylinder (back side).
            let phi = PI - (d / radius).asin();
            let so = f - radius * phi;
            let (px, py) = page_at(so);
            if inside(px, py) {
                return washed(pixel_clamped(t.outgoing, px, py), 0.75 + 0.25 * phi.sin());
            }
            // 3. Lower half (front side, shaded).
            let phi = (d / radius).asin();
            let so = f - radius * phi;
            let (px, py) = page_at(so);
            if inside(px, py) {
                let color = pixel_clamped(t.outgoing, px, py);
                let light = 1. - 0.45 * (phi / FRAC_PI_2);
                let [r, g, b, a] = color;
                return [r * light, g * light, b * light, a];
            }
        }
        if s >= f {
            return pixel_clamped(t.outgoing, x, y);
        }
        // 4. Revealed incoming clip, with a soft shadow along the curl.
        let incoming = pixel_clamped(t.incoming, x, y);
        if shadow {
            let light = 1.
                - 0.45
                    * (-(d - radius).max(0.) / (radius * 1.5)).exp()
                    * smoothstep(0., 0.05, progress)
                    * (1. - smoothstep(0.9, 1., progress));
            let [r, g, b, a] = incoming;
            return [r * light, g * light, b * light, a];
        }
        incoming
    })
}

fn wave(t: &Tx, out: &mut Out) -> Result<()> {
    let (dx, dy) = dir(t)?;
    let (w, h) = (t.width as f32, t.height as f32);
    let horizontal = dx != 0.;
    let amplitude = px(t, "amplitude")?;
    let wavelength = px(t, "wavelength")?.max(4.);
    let negative = dx + dy < 0.;
    let length = travel(t, dx, dy);
    let band = length * 0.25;
    let progress = t.progress;
    let edge = -band + progress * (length + 2. * band);
    let envelope = bell(progress);
    out.paint(|xi, yi| {
        let x = xi as f32 + 0.5;
        let y = yi as f32 + 0.5;
        let s = if horizontal {
            if negative { w - x } else { x }
        } else if negative {
            h - y
        } else {
            y
        };
        let across = if horizontal { y } else { x };
        let near = (-((s - edge) / band).powi(2)).exp();
        let displacement =
            amplitude * envelope * near * (TAU * (across / wavelength) - progress * 6.).sin();
        let (sx, sy) = if horizontal {
            (x + displacement, y + displacement * 0.3)
        } else {
            (x + displacement * 0.3, y + displacement)
        };
        let k = 1. - smoothstep(-band * 0.3, band * 0.3, s - edge);
        mix(
            sample_clamped(t.outgoing, sx, sy),
            sample_clamped(t.incoming, sx, sy),
            k,
        )
    })
}

fn slice(t: &Tx, out: &mut Out) -> Result<()> {
    let (w, h) = (t.width as f32, t.height as f32);
    let n = t.params.num("slices")?.round().clamp(1., 64.);
    let angle = t.params.angle("angle")?;
    let (ax, ay) = (angle.cos(), angle.sin()); // Slice direction (they slide along this).
    let (across_x, across_y) = (-ay, ax);
    let corners = [(0., 0.), (w, 0.), (0., h), (w, h)];
    let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
    for (cx, cy) in corners {
        let v = cx * across_x + cy * across_y;
        low = low.min(v);
        high = high.max(v);
    }
    let span = (high - low).max(1.);
    let distance = w * ax.abs() + h * ay.abs() + span * 0.2;
    let (shutter, samples) = shutter(t, 0.08)?;
    let progress = t.progress;
    out.paint(|xi, yi| {
        let x = xi as f32 + 0.5;
        let y = yi as f32 + 0.5;
        temporal(x, y, progress, shutter, samples, &|x, y, q| {
            let i = (((x * across_x + y * across_y) - low) / span * n)
                .floor()
                .clamp(0., n - 1.);
            let sign = if (i as u32).is_multiple_of(2) {
                1.
            } else {
                -1.
            };
            let qi = ((q - 0.3 * i / n.max(1.)) / 0.7).clamp(0., 1.);
            let offset = sign * distance * ease_in(qi);
            let (sx, sy) = (x - ax * offset, y - ay * offset);
            let zb = 1. + 0.05 * (1. - q);
            let incoming = sample_clamped(
                t.incoming,
                (x - w / 2.) / zb + w / 2.,
                (y - h / 2.) / zb + h / 2.,
            );
            if sx >= 0. && sx < w && sy >= 0. && sy < h {
                // Keep the slice's own band only (slices do not bleed into each other).
                let j = (((sx * across_x + sy * across_y) - low) / span * n)
                    .floor()
                    .clamp(0., n - 1.);
                if j == i {
                    return over(incoming, pixel_clamped(t.outgoing, sx, sy));
                }
            }
            incoming
        })
    })
}

fn motion_camera(t: &Tx, out: &mut Out) -> Result<()> {
    let (w, h) = (t.width as f32, t.height as f32);
    let zoom = t.params.fraction("zoom")?;
    let rotation = t.params.angle("rotation")?;
    let motion_blur = t.params.fraction("motion_blur")?;
    let progress = t.progress;
    let k = mid_mix(progress);
    // A pushes in and turns; B arrives from the same push, settling to rest.
    let (zoom_a, rotation_a) = (
        1. + zoom * ease_in(progress.min(0.7) / 0.7),
        rotation * ease_in(progress),
    );
    let (zoom_b, rotation_b) = (
        1. + zoom * (1. - ease_out(((progress - 0.3) / 0.7).clamp(0., 1.))),
        -rotation * (1. - ease_out(progress)),
    );
    let blur = motion_blur * bell(progress) * 0.35;
    let n = taps(blur * w * 0.3);
    out.paint(|xi, yi| {
        let x = xi as f32 + 0.5;
        let y = yi as f32 + 0.5;
        let sample = |image: &Image, zoom: f32, rotation: f32| {
            let (sin, cos) = (-rotation).sin_cos();
            let (ux, uy) = ((x - w / 2.) / zoom, (y - h / 2.) / zoom);
            let (lx, ly) = (ux * cos - uy * sin + w / 2., ux * sin + uy * cos + h / 2.);
            zoom_blur(image, lx, ly, w / 2., h / 2., blur, n)
        };
        let a = if k < 1. {
            sample(t.outgoing, zoom_a, rotation_a)
        } else {
            [0.; 4]
        };
        let b = if k > 0. {
            sample(t.incoming, zoom_b, rotation_b)
        } else {
            [0.; 4]
        };
        mix(a, b, k)
    })
}

fn motion_tween(t: &Tx, out: &mut Out) -> Result<()> {
    let (w, h) = (t.width as f32, t.height as f32);
    let scale = t.params.fraction("scale")?;
    let progress = t.progress;
    let e = ease_in_out(progress);
    let (zoom_a, zoom_b) = (
        (1. + scale * e).max(1e-3),
        (1. + scale * (e - 1.)).max(1e-3),
    );
    out.paint(|xi, yi| {
        let x = xi as f32 + 0.5;
        let y = yi as f32 + 0.5;
        let a = sample_clamped(
            t.outgoing,
            (x - w / 2.) / zoom_a + w / 2.,
            (y - h / 2.) / zoom_a + h / 2.,
        );
        let b = sample_clamped(
            t.incoming,
            (x - w / 2.) / zoom_b + w / 2.,
            (y - h / 2.) / zoom_b + h / 2.,
        );
        mix(a, b, e)
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn easings_hit_their_ends() {
        for value in [spring(0., 0.4), back_out(0., 1.)] {
            assert!(value.abs() < 1e-5);
        }
        assert!((spring(1., 0.4) - 1.).abs() < 1e-5);
        assert!((back_out(1., 1.) - 1.).abs() < 1e-5);
        assert!(back_out(0.8, 1.6) > 1., "overshoots");
    }

    #[test]
    fn card_geometry_refuses_frames_above_the_output_size_cap() {
        assert!(check_geometry(1920, 1080).is_ok());
        assert!(check_geometry(1, 1).is_ok());
        for (width, height) in [(0, 1080), (1920, 0), (8193, 8193), (usize::MAX, 1)] {
            assert!(
                matches!(
                    check_geometry(width, height),
                    Err(Error::Invalid("transition frame size"))
                ),
                "{width}x{height}"
            );
        }
    }
}
