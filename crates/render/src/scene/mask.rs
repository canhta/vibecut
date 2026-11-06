//! Polygon mask coverage in output pixels, in the linear premultiplied working space.
use crate::{Error, ExecutionHooks, Image, MemoryKind, Result, check};

/// One evaluated, flattened contour. Last point connects to first.
/// Coordinates are output pixels; feather/expansion are output-pixel distances.
/// At most 65,536 finite vertices with absolute coordinates <= 1e8 are admitted.
#[derive(Clone, Debug)]
pub struct FlatMask {
    pub pts: Vec<[f32; 2]>,
    pub feather: f32,
    pub expansion: f32,
    pub opacity: f32,
    pub inverted: bool,
}

/// The source distance falloff: hard edges have a one-pixel linear ramp;
/// feather >= 1 uses smoothstep over exactly `feather` pixels.
/// Inputs must be finite, with nonnegative feather.
#[inline]
fn falloff(s: f32, feather: f32) -> f32 {
    let u = (s / feather.max(1.0) + 0.5).clamp(0.0, 1.0);
    let smooth = u * u * (3.0 - 2.0 * u);
    u + (smooth - u) * feather.min(1.0)
}

#[inline]
fn seg_dist2(px: f32, py: f32, a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let (qx, qy) = (px - a[0], py - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 {
        ((qx * dx + qy * dy) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (ex, ey) = (qx - dx * t, qy - dy * t);
    ex * ex + ey * ey
}
/// Stroke layers have their own one-pixel antialiasing; mask feather affects
/// body coverage only. Distances are positive on the retained side.
fn stroke_coverage(s: f32, inside: f32, outside: f32) -> f32 {
    (falloff(s + outside, 0.) - falloff(s - inside, 0.)).max(0.)
}
type Edge = ([f32; 2], [f32; 2], f32, f32);

impl FlatMask {
    fn has_area(&self) -> bool {
        let Some(&a) = self.pts.first() else {
            return false;
        };
        let Some(&b) = self.pts.iter().find(|p| **p != a) else {
            return false;
        };
        self.pts
            .iter()
            .any(|p| (b[0] - a[0]) * (p[1] - a[1]) != (b[1] - a[1]) * (p[0] - a[0]))
    }

    fn validate(&self) -> Result<()> {
        if self.pts.len() > 65_536
            || self
                .pts
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > 1e8)
            || !self.feather.is_finite()
            || !(0.0..=1e6).contains(&self.feather)
            || !self.expansion.is_finite()
            || self.expansion.abs() > 1e6
            || !self.opacity.is_finite()
            || !(0.0..=1.0).contains(&self.opacity)
        {
            return Err(Error::Invalid("evaluated mask"));
        }
        Ok(())
    }
    /// Rasterize one mask into grayscale premultiplied working pixels.
    /// Empty/degenerate contours have zero coverage before invert. Checks
    /// cancellation each row and reserves image/scratch bytes before allocation.
    pub fn coverage(&self, width: u32, height: u32, hooks: &dyn ExecutionHooks) -> Result<Image> {
        self.rasterize(
            width,
            height,
            self.feather,
            0.0,
            |sd| {
                let c = falloff(sd + self.expansion, self.feather);
                self.opacity * if self.inverted { 1.0 - c } else { c }
            },
            hooks,
        )
    }
    /// Stroke coverage on the expanded/inverted boundary. `inside` and `outside`
    /// are nonnegative distances on the retained and removed sides respectively.
    /// (width,0), (width/2,width/2), (0,width) encode inside, centre, outside.
    pub fn stroke(
        &self,
        width: u32,
        height: u32,
        inside: f32,
        outside: f32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Image> {
        if [inside, outside]
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1e6).contains(v))
        {
            return Err(Error::Invalid("mask stroke width"));
        }
        let has_area = self.has_area();
        self.rasterize(
            width,
            height,
            0.,
            inside.max(outside),
            |sd| {
                if inside + outside == 0.0 || !has_area {
                    return 0.0;
                }
                let s = (sd + self.expansion) * if self.inverted { -1.0 } else { 1.0 };
                self.opacity * stroke_coverage(s, inside, outside)
            },
            hooks,
        )
    }
    fn rasterize(
        &self,
        width: u32,
        height: u32,
        edge_softness: f32,
        extra: f32,
        value: impl Fn(f32) -> f32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Image> {
        check(hooks)?;
        self.validate()?;
        let mut image = Image::new(width, height, [0.0; 4], hooks)?;
        let n = self.pts.len();
        let has_area = self.has_area();
        let band = self.expansion.abs() + edge_softness.max(1.0) * 0.5 + extra + 1.0;
        if !has_area {
            for row in image.pixels.chunks_mut(width as usize) {
                check(hooks)?;
                row.fill([value(-band).clamp(0., 1.); 4]);
            }
            return Ok(image);
        }
        let _scratch = hooks.reserve(
            MemoryKind::Compositor,
            n as u64 * (std::mem::size_of::<Edge>() + std::mem::size_of::<(f32, i32)>()) as u64,
        )?;
        let mut cross = Vec::<(f32, i32)>::new();
        cross.try_reserve_exact(n)?;
        let mut near = Vec::<Edge>::new();
        near.try_reserve_exact(n)?;
        for (y, row) in image.pixels.chunks_mut(width as usize).enumerate() {
            check(hooks)?;
            cross.clear();
            near.clear();
            let py = y as f32 + 0.5;
            if let Some(first) = self.pts.first() {
                for (&a, &b) in self
                    .pts
                    .iter()
                    .zip(self.pts.iter().skip(1).chain(std::iter::once(first)))
                {
                    if (a[1] <= py) != (b[1] <= py) {
                        let t = (py - a[1]) / (b[1] - a[1]);
                        cross.push((a[0] + (b[0] - a[0]) * t, if b[1] > a[1] { 1 } else { -1 }));
                    }
                    if py >= a[1].min(b[1]) - band && py <= a[1].max(b[1]) + band {
                        near.push((a, b, a[0].min(b[0]) - band, a[0].max(b[0]) + band));
                    }
                }
            }
            cross.sort_unstable_by(|p, q| p.0.total_cmp(&q.0));
            let mut crossings = cross.iter().peekable();
            let mut wind = 0;
            for (x, output) in row.iter_mut().enumerate() {
                if x.is_multiple_of(256) {
                    check(hooks)?;
                }
                let px = x as f32 + 0.5;
                while crossings.peek().is_some_and(|c| c.0 < px) {
                    if let Some(c) = crossings.next() {
                        wind += c.1;
                    }
                }
                let mut d2 = band * band;
                for &(a, b, x0, x1) in &near {
                    if px >= x0 && px <= x1 {
                        d2 = d2.min(seg_dist2(px, py, a, b));
                    }
                }
                let sd = if !has_area {
                    -band
                } else if wind != 0 {
                    d2.sqrt()
                } else {
                    -d2.sqrt()
                };
                *output = [value(sd).clamp(0.0, 1.0); 4];
            }
        }
        Ok(image)
    }
}

/// Scale all four premultiplied item channels by coverage, without converting alpha.
/// Geometry is checked before mutation; cancellation is checked each row.
pub fn scale_by(image: &mut Image, coverage: &Image, hooks: &dyn ExecutionHooks) -> Result<()> {
    same_size(image, coverage)?;
    for (row, mask) in image
        .pixels
        .chunks_mut(image.width as usize)
        .zip(coverage.pixels.chunks(coverage.width as usize))
    {
        check(hooks)?;
        for (p, c) in row.iter_mut().zip(mask) {
            for v in p {
                *v *= c[3];
            }
        }
    }
    Ok(())
}
fn same_size(a: &Image, b: &Image) -> Result<()> {
    if a.width != b.width || a.height != b.height {
        Err(Error::Invalid("mask image dimensions"))
    } else {
        Ok(())
    }
}

/// Output-pixel geometry after the project/placement transform has been evaluated.
#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub center: [f32; 2],
    pub size: [f32; 2],
    /// Clockwise radians in the y-down output coordinate system.
    pub rotation: f32,
}
/// A flattened built-in contour and the reservation retained for its geometry.
pub struct PreparedMask {
    pub mask: FlatMask,
    _lease: Box<dyn crate::Reservation>,
}
/// Build a contour with <=0.05 output-pixel chord error for cubic segments.
/// Supported contour identifiers are rectangle, cinematic-bars, ellipse, heart,
/// diamond and star. Split is an unbounded half-plane and uses `SplitMask`.
/// Size must be positive and finite, <=1e6 pixels; geometry must fit <=1e8 pixels.
pub fn prepare_builtin(
    kind: &str,
    geometry: Geometry,
    hooks: &dyn ExecutionHooks,
) -> Result<PreparedMask> {
    check(hooks)?;
    if geometry
        .center
        .iter()
        .any(|v| !v.is_finite() || v.abs() > 9e7)
        || geometry
            .size
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1e6).contains(v) || *v == 0.0)
        || !geometry.rotation.is_finite()
    {
        return Err(Error::Invalid("mask geometry"));
    }
    let vertex_bound = match kind {
        "rectangle" | "cinematic-bars" | "diamond" => 4,
        "star" => 10,
        "ellipse" | "heart" => {
            let segments = if kind == "ellipse" { 4 } else { 2 };
            segments
                * (75.0 * geometry.size[0].max(geometry.size[1]))
                    .sqrt()
                    .ceil()
                    .max(1.0) as usize
        }
        _ => return Err(Error::Unsupported("mask contour")),
    };
    if vertex_bound > 65_536 {
        return Err(Error::Invalid("mask flattening bound"));
    }
    let _lease = hooks.reserve(MemoryKind::Compositor, vertex_bound as u64 * 8)?;
    let mut pts = Vec::new();
    pts.try_reserve_exact(vertex_bound)?;
    let mut push = |p: [f32; 2]| -> Result<()> {
        if pts.len() >= vertex_bound {
            return Err(Error::Invalid("mask flattening bound"));
        }
        let (sin, cos) = geometry.rotation.sin_cos();
        let x = p[0] * geometry.size[0] * 0.5;
        let y = p[1] * geometry.size[1] * 0.5;
        pts.push([
            geometry.center[0] + cos * x - sin * y,
            geometry.center[1] + sin * x + cos * y,
        ]);
        Ok(())
    };
    let mut curve = |p: [[f32; 2]; 4]| -> Result<()> {
        let distance = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| {
            ((a[0] - 2. * b[0] + c[0]) * geometry.size[0] * 0.5)
                .hypot((a[1] - 2. * b[1] + c[1]) * geometry.size[1] * 0.5)
        };
        let bound = distance(p[0], p[1], p[2]).max(distance(p[1], p[2], p[3]));
        let steps = (0.75 * bound / 0.05).sqrt().ceil().max(1.) as usize;
        if steps > 65_536 {
            return Err(Error::Invalid("mask flattening bound"));
        }
        for n in 0..steps {
            let t = n as f32 / steps as f32;
            let u = 1. - t;
            push([
                u * u * u * p[0][0]
                    + 3. * u * u * t * p[1][0]
                    + 3. * u * t * t * p[2][0]
                    + t * t * t * p[3][0],
                u * u * u * p[0][1]
                    + 3. * u * u * t * p[1][1]
                    + 3. * u * t * t * p[2][1]
                    + t * t * t * p[3][1],
            ])?;
        }
        Ok(())
    };
    match kind {
        "ellipse" => {
            let k = 0.552_284_8;
            for p in [
                [[1., 0.], [1., k], [k, 1.], [0., 1.]],
                [[0., 1.], [-k, 1.], [-1., k], [-1., 0.]],
                [[-1., 0.], [-1., -k], [-k, -1.], [0., -1.]],
                [[0., -1.], [k, -1.], [1., -k], [1., 0.]],
            ] {
                curve(p)?;
            }
        }
        "heart" => {
            curve([[0., -0.475], [1., -1.225], [1., -0.125], [0., 0.725]])?;
            curve([[0., 0.725], [-1., -0.125], [-1., -1.225], [0., -0.475]])?;
        }
        "rectangle" | "cinematic-bars" => {
            for p in [[-1., -1.], [1., -1.], [1., 1.], [-1., 1.]] {
                push(p)?;
            }
        }
        "diamond" => {
            for p in [[0., -1.], [1., 0.], [0., 1.], [-1., 0.]] {
                push(p)?;
            }
        }
        "star" => {
            for n in 0..10 {
                let angle = n as f32 * std::f32::consts::PI / 5. - std::f32::consts::FRAC_PI_2;
                let r = if n % 2 == 0 { 1. } else { 0.45 };
                push([angle.cos() * r, angle.sin() * r])?;
            }
        }
        _ => return Err(Error::Unsupported("mask contour")),
    }
    let mask = FlatMask {
        pts,
        feather: 0.,
        expansion: 0.,
        opacity: 1.,
        inverted: false,
    };
    mask.validate()?;
    Ok(PreparedMask { mask, _lease })
}

/// Evaluated split line in output pixels. The unit normal points into the
/// retained half-plane; artificial canvas boundaries never feather or stroke.
pub struct SplitMask {
    pub center: [f32; 2],
    pub normal: [f32; 2],
    pub feather: f32,
    pub expansion: f32,
    pub opacity: f32,
    pub inverted: bool,
}
impl SplitMask {
    pub fn coverage(&self, width: u32, height: u32, hooks: &dyn ExecutionHooks) -> Result<Image> {
        self.rasterize(
            width,
            height,
            |s| {
                let c = falloff(s, self.feather);
                self.opacity * if self.inverted { 1. - c } else { c }
            },
            hooks,
        )
    }
    /// Inside/centre/outside follow `FlatMask::stroke` and share its falloff.
    pub fn stroke(
        &self,
        width: u32,
        height: u32,
        inside: f32,
        outside: f32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Image> {
        if [inside, outside]
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1e6).contains(v))
        {
            return Err(Error::Invalid("split mask stroke"));
        }
        self.rasterize(
            width,
            height,
            |s| {
                let s = s * if self.inverted { -1. } else { 1. };
                self.opacity * stroke_coverage(s, inside, outside)
            },
            hooks,
        )
    }
    fn rasterize(
        &self,
        width: u32,
        height: u32,
        value: impl Fn(f32) -> f32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Image> {
        check(hooks)?;
        if self.center.iter().any(|v| !v.is_finite() || v.abs() > 1e8)
            || self.normal.iter().any(|v| !v.is_finite())
            || (self.normal[0].hypot(self.normal[1]) - 1.).abs() > 1e-5
            || !self.feather.is_finite()
            || !(0.0..=1e6).contains(&self.feather)
            || !self.expansion.is_finite()
            || self.expansion.abs() > 1e6
            || !self.opacity.is_finite()
            || !(0.0..=1.).contains(&self.opacity)
        {
            return Err(Error::Invalid("split mask geometry"));
        }
        let mut image = Image::new(width, height, [0.; 4], hooks)?;
        for (y, row) in image.pixels.chunks_mut(width as usize).enumerate() {
            check(hooks)?;
            for (x, p) in row.iter_mut().enumerate() {
                let d = (x as f32 + 0.5 - self.center[0]) * self.normal[0]
                    + (y as f32 + 0.5 - self.center[1]) * self.normal[1]
                    + self.expansion;
                *p = [value(d).clamp(0., 1.); 4];
            }
        }
        Ok(image)
    }
}

/// A signed-distance raster prepared once from already-ingressed glyph coverage.
/// Text owns the distance transform. Colour/alpha transfer conversion is not repeated.
pub struct RasterMask {
    width: u32,
    height: u32,
    distance: Vec<f32>,
    _lease: Box<dyn crate::Reservation>,
}
impl RasterMask {
    pub fn from_image(image: &Image, hooks: &dyn ExecutionHooks) -> Result<Self> {
        check(hooks)?;
        // The text distance operation retains three area buffers and produces the
        // fourth; scratch holds f,d,v,z for the longer axis (usize is 8 bytes).
        let retained = u64::from(image.width) * u64::from(image.height) * 4;
        let _lease = hooks.reserve(MemoryKind::Compositor, retained)?;
        let peak = u64::from(image.width) * u64::from(image.height) * 12
            + u64::from(image.width.max(image.height)) * 24
            + 4;
        let transient = hooks.reserve(MemoryKind::Compositor, peak)?;
        let distance = text::coverage_signed_distance(
            image.width,
            image.height,
            image.pixels.iter().map(|p| p[3]),
            || hooks.cancelled(),
        )
        .map_err(|error| match error {
            text::RenderError::Cancelled => Error::Cancelled,
            error => Error::Raster(error),
        })?;
        check(hooks)?;
        drop(transient);
        Ok(Self {
            width: image.width,
            height: image.height,
            distance,
            _lease,
        })
    }
    /// Stroke the retained glyph boundary with the same inside/outside widths
    /// as polygon masks. The prepared distance field is reused for every pass.
    #[allow(clippy::too_many_arguments)]
    pub fn stroke(
        &self,
        expansion: f32,
        opacity: f32,
        inverted: bool,
        inside: f32,
        outside: f32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Image> {
        if [inside, outside]
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1e6).contains(v))
            || !expansion.is_finite()
            || expansion.abs() > 1e6
            || !opacity.is_finite()
            || !(0.0..=1.0).contains(&opacity)
        {
            return Err(Error::Invalid("raster mask stroke"));
        }
        let mut image = Image::new(self.width, self.height, [0.; 4], hooks)?;
        for (row, distances) in image
            .pixels
            .chunks_mut(self.width as usize)
            .zip(self.distance.chunks(self.width as usize))
        {
            check(hooks)?;
            for (p, d) in row.iter_mut().zip(distances) {
                let s = (-*d + expansion) * if inverted { -1. } else { 1. };
                *p = [opacity * stroke_coverage(s, inside, outside); 4];
            }
        }
        Ok(image)
    }
    /// Raster feather, expansion, opacity and invert share polygon falloff math.
    /// Distances and feather are output pixels; opacity admits finite 0..=1.
    pub fn coverage(
        &self,
        feather: f32,
        expansion: f32,
        opacity: f32,
        inverted: bool,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Image> {
        if !feather.is_finite()
            || !(0.0..=1e6).contains(&feather)
            || !expansion.is_finite()
            || expansion.abs() > 1e6
            || !opacity.is_finite()
            || !(0.0..=1.0).contains(&opacity)
        {
            return Err(Error::Invalid("raster mask appearance"));
        }
        let mut image = Image::new(self.width, self.height, [0.; 4], hooks)?;
        for (row, distances) in image
            .pixels
            .chunks_mut(self.width as usize)
            .zip(self.distance.chunks(self.width as usize))
        {
            check(hooks)?;
            for (p, d) in row.iter_mut().zip(distances) {
                let c = falloff(-*d + expansion, feather);
                *p = [opacity * if inverted { 1. - c } else { c }; 4];
            }
        }
        Ok(image)
    }
}

/// Starting coverage for the first active mode. `none` represents an entirely
/// inactive mask stack and starts fully covered. Modes are project-owned.
pub fn initial(mode: project::MaskMode) -> f32 {
    use project::MaskMode::*;
    match mode {
        None | Subtract | Intersect | Darken => 1.,
        Add | Lighten | Difference => 0.,
    }
}
/// Fold one mask into accumulated coverage, in persisted vector order.
/// The same scalar is written to all working channels; no alpha conversion occurs.
pub fn combine(
    accumulated: &mut Image,
    coverage: &Image,
    mode: project::MaskMode,
    hooks: &dyn ExecutionHooks,
) -> Result<()> {
    same_size(accumulated, coverage)?;
    use project::MaskMode::*;
    let operation: fn(f32, f32) -> f32 = match mode {
        None => |a, _| a,
        Add => |a, m| a + m - a * m,
        Subtract => |a, m| a * (1. - m),
        Intersect => |a, m| a * m,
        Lighten => f32::max,
        Darken => f32::min,
        Difference => |a, m| a + m - 2. * a * m,
    };
    for (row, mask) in accumulated
        .pixels
        .chunks_mut(accumulated.width as usize)
        .zip(coverage.pixels.chunks(coverage.width as usize))
    {
        check(hooks)?;
        for (p, m) in row.iter_mut().zip(mask) {
            *p = [operation(p[3], m[3]).clamp(0., 1.); 4];
        }
    }
    Ok(())
}
/// Paint one sRGB stroke colour through scalar coverage using the shared
/// linear-premultiplied source-over kernel. Call in mask order after clipping
/// the item; strokes are deliberately not clipped by the final combined mask.
pub fn paint_stroke(
    item: &mut Image,
    coverage: &Image,
    color: [u8; 4],
    hooks: &dyn ExecutionHooks,
) -> Result<()> {
    same_size(item, coverage)?;
    let color = crate::image::color(color);
    for (row, mask) in item
        .pixels
        .chunks_mut(item.width as usize)
        .zip(coverage.pixels.chunks(coverage.width as usize))
    {
        check(hooks)?;
        for (p, m) in row.iter_mut().zip(mask) {
            crate::image::over(p, color, m[3]);
        }
    }
    Ok(())
}

/// Flatten project-owned cubic paths into output geometry. Fill closes open paths,
/// matching the reference's nonzero-winding fill semantics.
fn prepare_path(
    path: &project::MaskPath,
    geometry: Geometry,
    hooks: &dyn ExecutionHooks,
) -> Result<PreparedMask> {
    let _lease = hooks.reserve(MemoryKind::Compositor, 65_536 * 8)?;
    let mut pts = Vec::new();
    pts.try_reserve_exact(65_536)?;
    let (sin, cos) = f64::from(geometry.rotation).sin_cos();
    let map = |p: [f64; 2]| {
        let x = (p[0] - 0.5) * f64::from(geometry.size[0]);
        let y = (p[1] - 0.5) * f64::from(geometry.size[1]);
        [
            f64::from(geometry.center[0]) + cos * x - sin * y,
            f64::from(geometry.center[1]) + sin * x + cos * y,
        ]
    };
    if path.vertices.len() >= 2
        && let Some(first) = path.vertices.first()
    {
        for (a, b) in path
            .vertices
            .iter()
            .zip(path.vertices.iter().skip(1).chain(std::iter::once(first)))
        {
            check(hooks)?;
            let p = [
                map(a.p),
                map([a.p[0] + a.t_out[0], a.p[1] + a.t_out[1]]),
                map([b.p[0] + b.t_in[0], b.p[1] + b.t_in[1]]),
                map(b.p),
            ];
            let bend = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
                (a[0] - 2. * b[0] + c[0]).hypot(a[1] - 2. * b[1] + c[1])
            };
            let straight = a.t_out == [0.; 2] && b.t_in == [0.; 2];
            let steps = if straight {
                1
            } else {
                (0.75 * bend(p[0], p[1], p[2]).max(bend(p[1], p[2], p[3])) / 0.05)
                    .sqrt()
                    .ceil()
                    .max(1.) as usize
            };
            if steps > 65_536 - pts.len() {
                return Err(Error::Invalid("mask flattening bound"));
            }
            for n in 0..steps {
                let t = n as f64 / steps as f64;
                let u = 1. - t;
                pts.push(std::array::from_fn(|k| {
                    (u * u * u * p[0].get(k).copied().unwrap_or(0.0)
                        + 3. * u * u * t * p[1].get(k).copied().unwrap_or(0.0)
                        + 3. * u * t * t * p[2].get(k).copied().unwrap_or(0.0)
                        + t * t * t * p[3].get(k).copied().unwrap_or(0.0))
                        as f32
                }));
            }
        }
    }
    let mask = FlatMask {
        pts,
        feather: 0.,
        expansion: 0.,
        opacity: 1.,
        inverted: false,
    };
    mask.validate()?;
    Ok(PreparedMask { mask, _lease })
}
enum Prepared {
    Polygon(PreparedMask),
    Split(SplitMask),
    Raster(RasterMask),
}
impl Prepared {
    fn coverage(
        &self,
        m: &project::Mask,
        scale: f32,
        w: u32,
        h: u32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Image> {
        match self {
            Self::Polygon(p) => p.mask.coverage(w, h, hooks),
            Self::Split(p) => p.coverage(w, h, hooks),
            Self::Raster(p) => p.coverage(
                m.feather as f32 * scale,
                m.expansion as f32 * scale,
                m.opacity as f32,
                m.inverted,
                hooks,
            ),
        }
    }
    fn stroke(
        &self,
        m: &project::Mask,
        scale: f32,
        w: u32,
        h: u32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Image> {
        let width = m.stroke.width as f32 * scale;
        let (inside, outside) = match m.stroke.align {
            project::StrokeAlign::Inside => (width, 0.),
            project::StrokeAlign::Center => (width * 0.5, width * 0.5),
            project::StrokeAlign::Outside => (0., width),
        };
        match self {
            Self::Polygon(p) => p.mask.stroke(w, h, inside, outside, hooks),
            Self::Split(p) => p.stroke(w, h, inside, outside, hooks),
            Self::Raster(p) => p.stroke(
                m.expansion as f32 * scale,
                m.opacity as f32,
                m.inverted,
                inside,
                outside,
                hooks,
            ),
        }
    }
}
/// Text mask content and typography handed to the caller's glyph rasterizer.
pub(crate) struct MaskText<'a> {
    pub content: &'a str,
    pub font: &'a str,
    pub weight: u16,
    pub italic: bool,
    pub letter_spacing: f32,
    pub line_height: f32,
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply(
    image: &mut Image,
    item: &project::Item,
    local: time::Tick,
    placement: crate::Placement,
    canvas: &project::Canvas,
    mut glyph: impl FnMut(MaskText<'_>, f32, Geometry) -> Result<std::sync::Arc<Image>>,
    hooks: &dyn ExecutionHooks,
) -> Result<()> {
    let Some(first) = item.masks.iter().find(|m| m.active()) else {
        return Ok(());
    };
    let w = image.width;
    let h = image.height;
    let dw = w as f32 * placement.destination.width as f32;
    let dh = h as f32 * placement.destination.height as f32;
    let scale = (dw / canvas.width as f32).min(dh / canvas.height as f32);
    let mut combined = Image::new(w, h, [initial(first.mode); 4], hooks)?;
    let mut strokes: Option<Image> = None;
    for mask in item.masks.iter().filter(|m| m.active()) {
        check(hooks)?;
        let extra = match &mask.shape {
            project::MaskShape::Text { content, .. } => content.as_str().len(),
            project::MaskShape::Path { path } => {
                path.vertices.len() * std::mem::size_of::<project::MaskVertex>()
            }
            _ => 0,
        };
        let _params = hooks.reserve(
            MemoryKind::Compositor,
            (extra + mask.id.len() + std::mem::size_of::<project::Mask>()) as u64,
        )?;
        let m = mask.at(&item.animations, local)?;
        let geometry = Geometry {
            center: [
                w as f32 * placement.destination.x as f32 + dw * (0.5 + m.center_x as f32),
                h as f32 * placement.destination.y as f32 + dh * (0.5 + m.center_y as f32),
            ],
            size: [
                dw * (m.width * m.scale) as f32,
                dh * (m.height * m.scale) as f32,
            ],
            rotation: (m.rotation as f32).to_radians(),
        };
        let prepared = match &m.shape {
            project::MaskShape::Split => {
                let (sin, cos) = geometry.rotation.sin_cos();
                Prepared::Split(SplitMask {
                    center: geometry.center,
                    normal: [cos, sin],
                    feather: m.feather as f32 * scale,
                    expansion: m.expansion as f32 * scale,
                    opacity: m.opacity as f32,
                    inverted: m.inverted,
                })
            }
            project::MaskShape::Text {
                content,
                font_size,
                font,
                weight,
                italic,
                letter_spacing,
                line_height,
            } => {
                let raster = glyph(
                    MaskText {
                        content: content.as_str(),
                        font: font.as_str(),
                        weight: *weight,
                        italic: *italic,
                        letter_spacing: *letter_spacing,
                        line_height: *line_height,
                    },
                    *font_size * m.scale as f32,
                    geometry,
                )?;
                let mut placed = Image::new(w, h, [0.; 4], hooks)?;
                let (sin, cos) = geometry.rotation.sin_cos();
                for (y, row) in placed.pixels.chunks_mut(w as usize).enumerate() {
                    check(hooks)?;
                    for (x, p) in row.iter_mut().enumerate() {
                        let dx = x as f32 + 0.5 - geometry.center[0];
                        let dy = y as f32 + 0.5 - geometry.center[1];
                        *p = crate::image::sample(
                            &raster,
                            cos * dx + sin * dy + raster.width as f32 * 0.5 - 0.5,
                            -sin * dx + cos * dy + raster.height as f32 * 0.5 - 0.5,
                        );
                    }
                }
                Prepared::Raster(RasterMask::from_image(&placed, hooks)?)
            }
            shape => {
                let mut g = geometry;
                if matches!(shape, project::MaskShape::CinematicBars) {
                    g.size[0] = dw.hypot(dh) * m.scale as f32;
                }
                let mut polygon = if let project::MaskShape::Path { path } = shape {
                    prepare_path(path, g, hooks)?
                } else {
                    prepare_builtin(shape.name(), g, hooks)?
                };
                polygon.mask.feather = m.feather as f32 * scale;
                polygon.mask.expansion = m.expansion as f32 * scale;
                polygon.mask.opacity = m.opacity as f32;
                polygon.mask.inverted = m.inverted;
                Prepared::Polygon(polygon)
            }
        };
        let coverage = prepared.coverage(&m, scale, w, h, hooks)?;
        combine(&mut combined, &coverage, m.mode, hooks)?;
        drop(coverage);
        if m.stroke.width > 0. {
            if strokes.is_none() {
                strokes = Some(Image::new(w, h, [0.; 4], hooks)?);
            }
            let coverage = prepared.stroke(&m, scale, w, h, hooks)?;
            if let Some(strokes) = strokes.as_mut() {
                paint_stroke(strokes, &coverage, m.stroke.color.0, hooks)?;
            }
        }
    }
    scale_by(image, &combined, hooks)?;
    drop(combined);
    if let Some(strokes) = strokes {
        for (row, src) in image
            .pixels
            .chunks_mut(w as usize)
            .zip(strokes.pixels.chunks(w as usize))
        {
            check(hooks)?;
            for (p, s) in row.iter_mut().zip(src) {
                crate::image::over(p, *s, 1.);
            }
        }
    }
    Ok(())
}
