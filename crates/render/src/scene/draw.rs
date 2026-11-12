//! Draws and blurs one prepared layer into the frame.

use super::{Fit, Placement};
use crate::image::over;
use crate::{Error, ExecutionHooks, Image, MemoryKind, Result, check};

struct Geometry {
    dx: f64,
    dy: f64,
    dw: f64,
    dh: f64,
    sx: f64,
    sy: f64,
    sw: f64,
    sh: f64,
}
impl Geometry {
    fn new(
        target_width: u32,
        target_height: u32,
        source_width: u32,
        source_height: u32,
        placement: Placement,
    ) -> Result<Self> {
        let mut dx = placement.destination.x * f64::from(target_width);
        let mut dy = placement.destination.y * f64::from(target_height);
        let mut dw = placement.destination.width * f64::from(target_width);
        let mut dh = placement.destination.height * f64::from(target_height);
        let mut sx = placement.crop.x * f64::from(source_width);
        let mut sy = placement.crop.y * f64::from(source_height);
        let mut sw = placement.crop.width * f64::from(source_width);
        let mut sh = placement.crop.height * f64::from(source_height);
        match placement.fit {
            Fit::Cover => {
                let scale = (dw / sw).max(dh / sh);
                let cw = dw / scale;
                let ch = dh / scale;
                sx += (sw - cw) * 0.5;
                sy += (sh - ch) * 0.5;
                sw = cw;
                sh = ch;
            }
            Fit::Contain => {
                let scale = (dw / sw).min(dh / sh);
                let w = sw * scale;
                let h = sh * scale;
                dx += (dw - w) * 0.5;
                dy += (dh - h) * 0.5;
                dw = w;
                dh = h;
            }
            Fit::Stretch => {}
        }
        if [dx, dy, dw, dh, sx, sy, sw, sh]
            .iter()
            .any(|n| !n.is_finite())
            || dw <= 0.0
            || dh <= 0.0
            || sw <= 0.0
            || sh <= 0.0
        {
            return Err(Error::Invalid("placement transform overflow"));
        }
        Ok(Self {
            dx,
            dy,
            dw,
            dh,
            sx,
            sy,
            sw,
            sh,
        })
    }
}
/// Clip-pixel gains before draw's optional source half reductions.
pub(super) fn clip_pixel_scale(
    target: [u32; 2],
    source: [u32; 2],
    placement: Placement,
) -> Result<[f64; 2]> {
    let g = Geometry::new(target[0], target[1], source[0], source[1], placement)?;
    Ok([g.dw / g.sw, g.dh / g.sh])
}
pub(super) fn horizontal_window(
    target_width: u32,
    target_height: u32,
    source_width: u32,
    source_height: u32,
    placement: Placement,
) -> Result<Option<std::ops::Range<u32>>> {
    let sx = f64::from(target_width) * placement.destination.width
        / (f64::from(source_width) * placement.crop.width);
    let sy = f64::from(target_height) * placement.destination.height
        / (f64::from(source_height) * placement.crop.height);
    let scale = match placement.fit {
        Fit::Cover => sx.max(sy),
        _ => sx.min(sy),
    };
    // Half reduction reads neighbors outside this level's sampling footprint.
    if scale <= 0.5 && source_width >= 4 && source_height >= 4 {
        return Ok(None);
    }
    let geometry = Geometry::new(
        target_width,
        target_height,
        source_width,
        source_height,
        placement,
    )?;
    let start = (geometry.sx.floor() - 2.0).clamp(0.0, f64::from(source_width)) as u32;
    let end = ((geometry.sx + geometry.sw).ceil() + 2.0).clamp(0.0, f64::from(source_width)) as u32;
    Ok((start < end && end - start < source_width / 2).then_some(start..end))
}

pub(super) fn draw(
    target: &mut Image,
    source: &Image,
    placement: Placement,
    hooks: &dyn ExecutionHooks,
) -> Result<()> {
    let mut reduced = None;
    let mut image = source;
    loop {
        let sx = f64::from(target.width) * placement.destination.width
            / (f64::from(image.width) * placement.crop.width);
        let sy = f64::from(target.height) * placement.destination.height
            / (f64::from(image.height) * placement.crop.height);
        let scale = match placement.fit {
            Fit::Cover => sx.max(sy),
            Fit::Contain => sx.min(sy),
            Fit::Stretch => sx.min(sy),
        };
        if scale > 0.5 || image.width < 4 || image.height < 4 {
            break;
        }
        let next = image.half(hooks)?;
        reduced = Some(next);
        image = reduced.as_ref().ok_or(Error::Invalid("image reduction"))?;
    }
    let Geometry {
        dx,
        dy,
        dw,
        dh,
        sx,
        sy,
        sw,
        sh,
    } = Geometry::new(
        target.width,
        target.height,
        image.width,
        image.height,
        placement,
    )?;
    let x0 = dx.floor().clamp(0.0, f64::from(target.width)) as usize;
    let x1 = (dx + dw).ceil().clamp(0.0, f64::from(target.width)) as usize;
    let y0 = dy.floor().clamp(0.0, f64::from(target.height)) as usize;
    let y1 = (dy + dh).ceil().clamp(0.0, f64::from(target.height)) as usize;
    // The separable transform is constant across rows. Compute horizontal
    // coordinates once, and resolve vertical neighbors once per row.
    let _axis = hooks.reserve(
        MemoryKind::Compositor,
        ((x1 - x0) * std::mem::size_of::<Option<(i64, f32)>>()) as u64,
    )?;
    let clamp = |p: f64, start: f64, len: f64| {
        if len < 1.0 {
            start + len * 0.5
        } else {
            p.clamp(start + 0.5, start + len - 0.5)
        }
    };
    let mut columns = crate::vector(x1 - x0, None)?;
    for (x, column) in (x0..x1).zip(&mut columns) {
        let px = x as f64 + 0.5;
        if px >= dx && px < dx + dw {
            let fx = clamp(sx + (px - dx) * sw / dw, sx, sw) - 0.5;
            let ix = fx.floor() as i64;
            *column = Some((ix, (fx - ix as f64) as f32));
        }
    }
    let row = |y: i64| {
        if y < 0 || y >= i64::from(image.height) {
            &[][..]
        } else {
            let start = y as usize * image.width as usize;
            image
                .pixels
                .get(start..start + image.width as usize)
                .unwrap_or(&[])
        }
    };
    let target_width = target.width as usize;
    let target_rows = target
        .pixels
        .get_mut(y0 * target_width..y1 * target_width)
        .ok_or(Error::Invalid("compositor rows"))?;
    crate::image::row_chunks(target_rows, target_width, hooks, |start, target_rows| {
        let _rows = hooks.reserve(
            MemoryKind::Compositor,
            ((x1 - x0) * 2 * std::mem::size_of::<[f32; 4]>()) as u64,
        )?;
        let mut top_samples = crate::vector(x1 - x0, [0.0; 4])?;
        let mut bottom_samples = crate::vector(x1 - x0, [0.0; 4])?;
        let mut top_index = None;
        let mut bottom_index = None;
        for (offset, target_row) in target_rows.chunks_mut(target_width).enumerate() {
            let y = y0 + start / target_width + offset;
            check(hooks)?;
            let py = y as f64 + 0.5;
            if py < dy || py >= dy + dh {
                continue;
            }
            let fy = clamp(sy + (py - dy) * sh / dh, sy, sh) - 0.5;
            let iy = fy.floor() as i64;
            let ty = (fy - iy as f64) as f32;
            // Adjacent destination rows usually share a source row. Cache only
            // horizontal interpolation; vertical interpolation keeps its original
            // operation order and rounding for every pixel.
            if bottom_index == Some(iy) {
                std::mem::swap(&mut top_samples, &mut bottom_samples);
                std::mem::swap(&mut top_index, &mut bottom_index);
            }
            if top_index != Some(iy) {
                horizontal_samples(&mut top_samples, row(iy), &columns);
                top_index = Some(iy);
            }
            if bottom_index != Some(iy + 1) {
                horizontal_samples(&mut bottom_samples, row(iy + 1), &columns);
                bottom_index = Some(iy + 1);
            }
            let destination = target_row
                .get_mut(x0..x1)
                .ok_or(Error::Invalid("compositor row"))?;
            for (((pixel, column), top), bottom) in destination
                .iter_mut()
                .zip(&columns)
                .zip(&top_samples)
                .zip(&bottom_samples)
            {
                if column.is_none() {
                    continue;
                }
                let source = std::array::from_fn(|k| {
                    let top = top.get(k).copied().unwrap_or(0.0);
                    let bottom = bottom.get(k).copied().unwrap_or(0.0);
                    top + (bottom - top) * ty
                });
                over(pixel, source, placement.opacity);
            }
        }
        Ok(())
    })?;
    drop(reduced);
    Ok(())
}
fn horizontal_samples(output: &mut [[f32; 4]], row: &[[f32; 4]], columns: &[Option<(i64, f32)>]) {
    for (sample, column) in output.iter_mut().zip(columns) {
        let Some((ix, tx)) = *column else { continue };
        let a = row.get(ix as usize).copied().unwrap_or([0.0; 4]);
        let b = row.get((ix + 1) as usize).copied().unwrap_or([0.0; 4]);
        *sample = std::array::from_fn(|k| {
            let a = a.get(k).copied().unwrap_or(0.0);
            let b = b.get(k).copied().unwrap_or(0.0);
            a + (b - a) * tx
        });
    }
}

pub(super) fn blur(image: &mut Image, radius: u32, hooks: &dyn ExecutionHooks) -> Result<()> {
    if radius == 0 {
        return Ok(());
    }
    let mut temp = Image::new(image.width, image.height, [0.0; 4], hooks)?;
    let r = i64::from(radius);
    let divisor = (2 * radius + 1) as f32;
    for vertical in [false, true] {
        let (source, dest) = if vertical {
            (&temp, &mut *image)
        } else {
            (&*image, &mut temp)
        };
        let outer = if vertical {
            source.width
        } else {
            source.height
        };
        let inner = if vertical {
            source.height
        } else {
            source.width
        };
        for line in 0..outer {
            check(hooks)?;
            let pixel = |p: i64| {
                let p = p.clamp(0, i64::from(inner) - 1);
                if vertical {
                    source.at(i64::from(line), p)
                } else {
                    source.at(p, i64::from(line))
                }
            };
            let mut sum = [0.0f32; 4];
            for p in -r..=r {
                for (s, v) in sum.iter_mut().zip(pixel(p)) {
                    *s += v;
                }
            }
            for n in 0..inner {
                if n > 0 {
                    for ((s, a), b) in sum
                        .iter_mut()
                        .zip(pixel(i64::from(n) - r - 1))
                        .zip(pixel(i64::from(n) + r))
                    {
                        *s += b - a;
                    }
                }
                let index = if vertical {
                    n as usize * source.width as usize + line as usize
                } else {
                    line as usize * source.width as usize + n as usize
                };
                let value = dest
                    .pixels
                    .get_mut(index)
                    .ok_or(Error::Invalid("blur index"))?;
                let alpha = (sum.get(3).copied().unwrap_or(0.0) / divisor).clamp(0.0, 1.0);
                for (channel, (d, s)) in value.iter_mut().zip(sum).enumerate() {
                    *d = if channel == 3 {
                        alpha
                    } else {
                        (s / divisor).clamp(0.0, alpha)
                    };
                }
            }
        }
    }
    Ok(())
}

/// Resolved graphic paint: fill, stroke, stroke width, alignment, corner radius.
type GraphicAppearance = ([f32; 4], [f32; 4], f64, project::StrokeAlign, f64);

/// Resolve appearance at the real item clock; keyed colors are already linear.
pub(super) fn graphic_appearance(
    item: &project::Item,
    local: time::Tick,
) -> Result<GraphicAppearance> {
    let project::ItemParams::Graphic(params) = &item.content else {
        return Err(Error::Invalid("graphic item"));
    };
    let (fill, stroke, width, align, radius) = params.appearance();
    let value = |name: &str| -> Result<Option<project::ParamValue>> {
        Ok(item.animations.value_at(
            &project::AnimationPath::new(format!("params.{name}"))?,
            local,
        )?)
    };
    let color = |name, fallback: project::Color| -> Result<[f32; 4]> {
        if let Some(project::ParamValue::Color(c)) = value(name)? {
            let a = c[3] as f32;
            Ok([c[0] as f32 * a, c[1] as f32 * a, c[2] as f32 * a, a])
        } else {
            Ok(crate::image::color(fallback.0))
        }
    };
    let scalar = |name, fallback: f64, max| -> Result<f64> {
        Ok(if let Some(project::ParamValue::Scalar(v)) = value(name)? {
            v.clamp(0., max)
        } else {
            fallback
        })
    };
    let align = match value("strokeAlign")? {
        Some(project::ParamValue::Text(t)) => match t.as_str() {
            "inside" => project::StrokeAlign::Inside,
            "outside" => project::StrokeAlign::Outside,
            _ => project::StrokeAlign::Center,
        },
        _ => align,
    };
    Ok((
        color("fill", fill)?,
        color("stroke", stroke)?,
        scalar("strokeWidth", width, 64.)?,
        align,
        scalar("cornerRadius", radius, 50.)?,
    ))
}
pub(super) fn draw_graphic(
    target: &mut Image,
    item: &project::Item,
    local: time::Tick,
    placement: Placement,
    hooks: &dyn ExecutionHooks,
) -> Result<()> {
    use super::mask::{Geometry as MaskGeometry, prepare_builtin, prepare_rounded_rectangle};
    let (fill, stroke, width, align, radius) = graphic_appearance(item, local)?;
    let r = placement.destination;
    let dw = r.width * f64::from(target.width);
    let dh = r.height * f64::from(target.height);
    let inset = if align == project::StrokeAlign::Center {
        width / 2.
    } else {
        0.
    };
    let extent = (512. - 2. * inset).max(1.) / 512.;
    let geometry = MaskGeometry {
        center: [
            ((r.x + r.width / 2.) * f64::from(target.width)) as f32,
            ((r.y + r.height / 2.) * f64::from(target.height)) as f32,
        ],
        size: [(dw * extent) as f32, (dh * extent) as f32],
        rotation: 0.,
    };
    let contour = if matches!(
        item.content,
        project::ItemParams::Graphic(project::GraphicParams::Rectangle { .. })
    ) {
        prepare_rounded_rectangle(geometry, (radius / 50.) as f32, hooks)?
    } else {
        prepare_builtin("ellipse", geometry, hooks)?
    };
    let paint = |target: &mut Image, coverage: Image, color: [f32; 4]| -> Result<()> {
        for (dst, src) in target
            .pixels
            .chunks_mut(target.width as usize)
            .zip(coverage.pixels.chunks(target.width as usize))
        {
            check(hooks)?;
            for (x, (dst, src)) in dst.iter_mut().zip(src).enumerate() {
                if x % 256 == 0 {
                    check(hooks)?;
                }
                over(dst, color, src[3] * placement.opacity);
            }
        }
        Ok(())
    };
    if fill[3] > 0. {
        paint(
            target,
            contour.mask.coverage(target.width, target.height, hooks)?,
            fill,
        )?;
    }
    if stroke[3] > 0. && width > 0. {
        let w = (width * dw.min(dh) / 512.) as f32;
        let (inside, outside) = match align {
            project::StrokeAlign::Inside => (w, 0.),
            project::StrokeAlign::Center => (w / 2., w / 2.),
            project::StrokeAlign::Outside => (0., w),
        };
        paint(
            target,
            contour
                .mask
                .stroke(target.width, target.height, inside, outside, hooks)?,
            stroke,
        )?;
    }
    Ok(())
}
